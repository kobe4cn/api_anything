use api_anything_common::error::AppError;
use api_anything_common::models::SandboxSession;
use api_anything_gateway::dispatcher::BackendDispatcher;
use api_anything_gateway::types::{GatewayRequest, GatewayResponse};
use axum::http::{HeaderValue, Method};
use serde_json::json;

pub struct ProxyLayer;

impl ProxyLayer {
    /// 在转发前注入 X-Sandbox-Tenant header，确保后端能按租户隔离资源；
    /// read_only 模式下拒绝非 GET 请求，防止沙箱会话意外修改生产数据
    pub async fn proxy(
        dispatcher: &BackendDispatcher,
        session: &SandboxSession,
        mut gateway_req: GatewayRequest,
    ) -> Result<GatewayResponse, AppError> {
        // read_only 检查在租户 header 注入之前，确保只读约束优先于转发逻辑生效
        if session.config.get("read_only") == Some(&json!(true))
            && gateway_req.method != Method::GET
        {
            return Err(AppError::BadRequest(
                "Sandbox session is read-only, only GET requests allowed".into(),
            ));
        }

        // tenant_id 注入到 header 而非 query param，因为后端服务通常通过 header 做租户路由，
        // 且 header 不会出现在访问日志的 URL 字段中，降低敏感信息泄露风险
        if let Ok(val) = HeaderValue::from_str(&session.tenant_id) {
            gateway_req.headers.insert("X-Sandbox-Tenant", val);
        }

        dispatcher.dispatch(gateway_req).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api_anything_gateway::adapter::{BoxFuture, ProtocolAdapter};
    use api_anything_gateway::dispatcher::ProtectionStack;
    use api_anything_gateway::types::*;
    use axum::http::HeaderMap;
    use std::collections::HashMap;
    use std::time::Duration;

    struct MockOkAdapter;

    impl ProtocolAdapter for MockOkAdapter {
        fn transform_request(&self, _: &GatewayRequest) -> Result<BackendRequest, AppError> {
            Ok(BackendRequest {
                endpoint: "mock".into(),
                method: Method::POST,
                headers: HeaderMap::new(),
                body: None,
                protocol_params: HashMap::new(),
            })
        }

        fn execute<'a>(
            &'a self,
            _: &'a BackendRequest,
        ) -> BoxFuture<'a, Result<BackendResponse, AppError>> {
            Box::pin(async {
                Ok(BackendResponse {
                    status_code: 200,
                    headers: HeaderMap::new(),
                    body: br#"{"ok":true}"#.to_vec(),
                    is_success: true,
                    duration_ms: 10,
                })
            })
        }

        fn transform_response(&self, resp: &BackendResponse) -> Result<GatewayResponse, AppError> {
            Ok(GatewayResponse {
                status_code: 200,
                headers: HashMap::new(),
                body: serde_json::from_slice(&resp.body).unwrap(),
            })
        }

        fn name(&self) -> &str {
            "mock"
        }
    }

    fn make_dispatcher() -> BackendDispatcher {
        BackendDispatcher::new(
            Box::new(MockOkAdapter),
            ProtectionStack::new(
                1000,
                100,
                50.0,
                Duration::from_secs(30),
                Duration::from_secs(60),
                3,
                Duration::from_secs(30),
            ),
        )
    }

    fn make_session(read_only: bool) -> SandboxSession {
        use api_anything_common::models::SandboxMode;
        use uuid::Uuid;
        SandboxSession {
            id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            tenant_id: "test-tenant".into(),
            mode: SandboxMode::Proxy,
            config: if read_only {
                json!({"read_only": true})
            } else {
                json!({})
            },
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            created_at: chrono::Utc::now(),
        }
    }

    fn make_request(method: Method) -> GatewayRequest {
        use uuid::Uuid;
        GatewayRequest {
            route_id: Uuid::new_v4(),
            method,
            path: "/test".into(),
            headers: HeaderMap::new(),
            query_params: HashMap::new(),
            path_params: HashMap::new(),
            body: None,
            trace_id: "t".into(),
        }
    }

    #[tokio::test]
    async fn read_only_blocks_post() {
        let dispatcher = make_dispatcher();
        let session = make_session(true);
        let req = make_request(Method::POST);
        let result = ProxyLayer::proxy(&dispatcher, &session, req).await;
        assert!(result.is_err());
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    #[tokio::test]
    async fn read_only_allows_get() {
        let dispatcher = make_dispatcher();
        let session = make_session(true);
        let req = make_request(Method::GET);
        let result = ProxyLayer::proxy(&dispatcher, &session, req).await;
        assert!(result.is_ok());
    }
}
