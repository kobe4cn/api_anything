use api_anything_common::error::ProblemDetail;
use axum::response::IntoResponse;

// fallback 兜底所有未匹配的路由，统一返回 RFC 7807 格式的 404，
// 避免框架默认的纯文本响应破坏 API 一致性
pub async fn fallback() -> impl IntoResponse {
    ProblemDetail {
        error_type: "about:blank".to_string(),
        title: "Not Found".to_string(),
        status: 404,
        detail: Some("The requested resource was not found".to_string()),
        instance: None,
    }
}
