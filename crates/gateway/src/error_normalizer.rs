use crate::types::BackendResponse;
use api_anything_common::error::AppError;
use std::time::Duration;

pub struct ErrorNormalizer;

impl ErrorNormalizer {
    /// 将后端响应统一映射为 AppError，覆盖三种错误场景：
    /// HTTP 4xx/5xx、SOAP Fault（HTTP 200 但语义失败）、其他非成功响应
    pub fn normalize(resp: &BackendResponse) -> Result<(), AppError> {
        // HTTP 层面的错误直接转换，status 保留用于调试
        if resp.status_code >= 400 {
            let body_text = String::from_utf8_lossy(&resp.body);
            return Err(AppError::BackendError {
                status: resp.status_code,
                detail: body_text.to_string(),
            });
        }

        // SOAP 协议在 HTTP 200 下通过 Fault 元素表达错误，需单独检测
        if !resp.is_success {
            let body_text = String::from_utf8_lossy(&resp.body);
            if body_text.contains("<soap:Fault>") || body_text.contains("<Fault>") {
                let detail = Self::extract_soap_fault(&body_text)
                    .unwrap_or_else(|| body_text.to_string());
                return Err(AppError::BackendError { status: 502, detail });
            }
            return Err(AppError::BackendError {
                status: 502,
                detail: body_text.to_string(),
            });
        }

        Ok(())
    }

    pub fn timeout_error(timeout: Duration) -> AppError {
        AppError::BackendTimeout { timeout_ms: timeout.as_millis() as u64 }
    }

    pub fn connection_error(detail: &str) -> AppError {
        AppError::BackendUnavailable(detail.to_string())
    }

    /// 从 SOAP Fault 体中提取 faultstring，提升错误消息的可读性
    fn extract_soap_fault(body: &str) -> Option<String> {
        let start = body.find("<faultstring>")? + "<faultstring>".len();
        let end = body[start..].find("</faultstring>")? + start;
        Some(body[start..end].to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    #[test]
    fn normalizes_http_error_status() {
        let resp = BackendResponse {
            status_code: 500,
            headers: HeaderMap::new(),
            body: b"Internal Server Error".to_vec(),
            is_success: false,
            duration_ms: 100,
        };
        assert!(ErrorNormalizer::normalize(&resp).is_err());
    }

    #[test]
    fn passes_through_successful_response() {
        let resp = BackendResponse {
            status_code: 200,
            headers: HeaderMap::new(),
            body: b"OK".to_vec(),
            is_success: true,
            duration_ms: 50,
        };
        assert!(ErrorNormalizer::normalize(&resp).is_ok());
    }

    #[test]
    fn normalizes_soap_fault() {
        let soap_fault = r#"<soap:Fault><faultcode>soap:Server</faultcode><faultstring>Order not found</faultstring></soap:Fault>"#;
        let resp = BackendResponse {
            status_code: 200,
            headers: HeaderMap::new(),
            body: soap_fault.as_bytes().to_vec(),
            is_success: false,
            duration_ms: 100,
        };
        let err = ErrorNormalizer::normalize(&resp).unwrap_err();
        let err_str = format!("{}", err);
        assert!(err_str.contains("Order not found"));
    }

    #[test]
    fn creates_timeout_error() {
        let err = ErrorNormalizer::timeout_error(Duration::from_secs(30));
        assert!(matches!(err, AppError::BackendTimeout { timeout_ms: 30000 }));
    }

    #[test]
    fn creates_connection_error() {
        let err = ErrorNormalizer::connection_error("Connection refused");
        assert!(matches!(err, AppError::BackendUnavailable(_)));
    }
}
