use crate::adapter::{BoxFuture, ProtocolAdapter};
use crate::types::{BackendRequest, BackendResponse, GatewayRequest, GatewayResponse};
use crate::xml_json::{SoapXmlBuilder, SoapXmlParser};
use api_anything_common::error::AppError;
use axum::http::{HeaderMap, Method};
use std::collections::HashMap;

/// SOAP 端点的静态配置，在路由绑定时确定，运行期不可变；
/// 将配置与 adapter 实例分离便于单元测试构造不同场景
#[derive(Debug, Clone)]
pub struct SoapConfig {
    pub endpoint_url: String,
    /// HTTP Header `SOAPAction` 的值，部分 SOAP 服务依赖它进行路由
    pub soap_action: String,
    /// WSDL 中的操作名，用于包裹请求体的 XML 元素名称
    pub operation_name: String,
    /// WSDL targetNamespace，生成的 Envelope 中 `xmlns:ns` 的值
    pub namespace: String,
}

/// 实现了 ProtocolAdapter 的 SOAP 适配器；
/// 负责 JSON→SOAP XML→HTTP POST→SOAP XML→JSON 的完整转换链路
pub struct SoapAdapter {
    config: SoapConfig,
    client: reqwest::Client,
}

impl SoapAdapter {
    pub fn new(config: SoapConfig) -> Self {
        Self {
            config,
            // 使用默认 Client 配置即可，超时由 BackendDispatcher 的 ProtectionStack 统一管理
            client: reqwest::Client::new(),
        }
    }
}

impl ProtocolAdapter for SoapAdapter {
    /// 将 GatewayRequest 的 JSON body 序列化为 SOAP Envelope，
    /// 并设置 SOAP 协议所需的两个关键 HTTP Header
    fn transform_request(&self, req: &GatewayRequest) -> Result<BackendRequest, AppError> {
        let empty_body = serde_json::Value::Object(serde_json::Map::new());
        let body_json = req.body.as_ref().unwrap_or(&empty_body);

        let envelope = SoapXmlBuilder::build_envelope(
            &self.config.soap_action,
            &self.config.operation_name,
            &self.config.namespace,
            body_json,
        );

        let mut headers = HeaderMap::new();
        // SOAP 1.1 规范要求 Content-Type 为 text/xml；charset 防止中文等非 ASCII 内容乱码
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            "text/xml; charset=utf-8"
                .parse()
                .map_err(|e| AppError::Internal(format!("Invalid content-type header: {e}")))?,
        );
        // SOAPAction 是 SOAP 1.1 的强制 Header，部分 WS-Security 实现也依赖它做鉴权
        headers.insert(
            "SOAPAction",
            self.config
                .soap_action
                .parse()
                .map_err(|e| AppError::Internal(format!("Invalid SOAPAction header: {e}")))?,
        );

        Ok(BackendRequest {
            endpoint: self.config.endpoint_url.clone(),
            method: Method::POST,
            headers,
            body: Some(envelope.into_bytes()),
            protocol_params: HashMap::new(),
        })
    }

    /// 通过 reqwest 向 SOAP 端点发送请求；
    /// 将 HTTP 层错误统一包装为 AppError::BackendUnavailable，
    /// 业务层错误（SOAP Fault）由 ErrorNormalizer 在 dispatcher 层处理
    fn execute<'a>(&'a self, req: &'a BackendRequest) -> BoxFuture<'a, Result<BackendResponse, AppError>> {
        Box::pin(async move {
            let start = std::time::Instant::now();

            let body_bytes = req.body.clone().unwrap_or_default();

            let mut request_builder = self
                .client
                .request(req.method.clone(), &req.endpoint);

            // 转发所有协议层设置的 Header（Content-Type、SOAPAction 等）
            for (name, value) in &req.headers {
                request_builder = request_builder.header(name, value);
            }

            let response = request_builder
                .body(body_bytes)
                .send()
                .await
                .map_err(|e| AppError::BackendUnavailable(format!("SOAP request failed: {e}")))?;

            let duration_ms = start.elapsed().as_millis() as u64;
            let status_code = response.status().as_u16();
            let is_success = response.status().is_success();
            let headers = response.headers().clone();

            let body = response
                .bytes()
                .await
                .map_err(|e| AppError::BackendUnavailable(format!("Failed to read SOAP response body: {e}")))?
                .to_vec();

            Ok(BackendResponse {
                status_code,
                headers,
                body,
                is_success,
                duration_ms,
            })
        })
    }

    /// 将 SOAP XML 响应体反序列化为 JSON；
    /// 解析失败时返回 Internal 错误而非 BackendError，
    /// 因为这属于适配器协议层面的问题而非后端业务错误
    fn transform_response(&self, resp: &BackendResponse) -> Result<GatewayResponse, AppError> {
        let xml = std::str::from_utf8(&resp.body)
            .map_err(|e| AppError::Internal(format!("SOAP response is not valid UTF-8: {e}")))?;

        let body = SoapXmlParser::parse_response(xml)
            .map_err(|e| AppError::Internal(format!("Failed to parse SOAP XML response: {e}")))?;

        // 将 HeaderMap 转为普通 HashMap 以匹配 GatewayResponse 的类型
        let headers = resp
            .headers
            .iter()
            .filter_map(|(k, v)| {
                v.to_str().ok().map(|vs| (k.to_string(), vs.to_string()))
            })
            .collect();

        Ok(GatewayResponse {
            status_code: resp.status_code,
            headers,
            body,
        })
    }

    fn name(&self) -> &str {
        "soap"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use axum::http::{HeaderMap, Method};
    use std::collections::HashMap;
    use uuid::Uuid;

    #[test]
    fn transform_request_builds_soap_envelope() {
        let adapter = SoapAdapter::new(SoapConfig {
            endpoint_url: "http://example.com/calculator".to_string(),
            soap_action: "http://example.com/calculator/Add".to_string(),
            operation_name: "Add".to_string(),
            namespace: "http://example.com/calculator".to_string(),
        });

        let req = GatewayRequest {
            route_id: Uuid::new_v4(),
            method: Method::POST,
            path: "/api/v1/calculator/add".to_string(),
            headers: HeaderMap::new(),
            query_params: HashMap::new(),
            path_params: HashMap::new(),
            body: Some(serde_json::json!({"a": 1, "b": 2})),
            trace_id: "test".to_string(),
        };

        let backend_req = adapter.transform_request(&req).unwrap();
        assert_eq!(backend_req.endpoint, "http://example.com/calculator");
        let body = String::from_utf8(backend_req.body.unwrap()).unwrap();
        assert!(body.contains("<soap:Envelope"));
        assert!(body.contains("<a>1</a>"));
    }

    #[test]
    fn transform_response_parses_soap_xml() {
        let adapter = SoapAdapter::new(SoapConfig {
            endpoint_url: "http://example.com".to_string(),
            soap_action: "test".to_string(),
            operation_name: "Add".to_string(),
            namespace: "http://example.com".to_string(),
        });

        let soap_response = r#"<?xml version="1.0"?>
        <soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body><AddResponse xmlns="http://example.com"><result>42</result></AddResponse></soap:Body>
        </soap:Envelope>"#;

        let backend_resp = BackendResponse {
            status_code: 200,
            headers: HeaderMap::new(),
            body: soap_response.as_bytes().to_vec(),
            is_success: true,
            duration_ms: 50,
        };

        let resp = adapter.transform_response(&backend_resp).unwrap();
        assert_eq!(resp.status_code, 200);
        assert_eq!(resp.body["result"], "42");
    }
}
