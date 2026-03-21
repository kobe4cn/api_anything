use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

// RFC 7807 标准错误响应体，Content-Type 为 application/problem+json，
// 让 API 消费方能够通过 type/title/status 字段以机器可读方式处理错误
#[derive(Debug, Serialize)]
pub struct ProblemDetail {
    #[serde(rename = "type")]
    pub error_type: String,
    pub title: String,
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
}

impl ProblemDetail {
    pub fn not_found(detail: impl Into<String>) -> Self {
        Self {
            error_type: "about:blank".into(),
            title: "Not Found".into(),
            status: 404,
            detail: Some(detail.into()),
            instance: None,
        }
    }
    pub fn bad_request(detail: impl Into<String>) -> Self {
        Self {
            error_type: "about:blank".into(),
            title: "Bad Request".into(),
            status: 400,
            detail: Some(detail.into()),
            instance: None,
        }
    }
    pub fn internal(detail: impl Into<String>) -> Self {
        Self {
            error_type: "about:blank".into(),
            title: "Internal Server Error".into(),
            status: 500,
            detail: Some(detail.into()),
            instance: None,
        }
    }
}

impl IntoResponse for ProblemDetail {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = serde_json::to_string(&self).unwrap_or_default();
        (status, [("content-type", "application/problem+json")], body).into_response()
    }
}

// AppError 作为所有服务层的统一错误类型，通过 IntoResponse 直接转为 HTTP 响应，
// 避免在 handler 层手动构造错误格式
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Bad request: {0}")]
    BadRequest(String),
    // 数据库错误通过 #[from] 自动转换，隐藏底层细节，仅在日志中记录原始错误
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Internal error: {0}")]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::NotFound(msg) => ProblemDetail::not_found(msg).into_response(),
            AppError::BadRequest(msg) => ProblemDetail::bad_request(msg).into_response(),
            AppError::Database(e) => {
                // 数据库错误不暴露给客户端，只写入结构化日志供内部排查
                tracing::error!(error = %e, "Database error");
                ProblemDetail::internal("Database error").into_response()
            }
            AppError::Internal(msg) => ProblemDetail::internal(msg).into_response(),
        }
    }
}
