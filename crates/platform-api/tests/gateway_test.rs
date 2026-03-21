use axum::http::StatusCode;
mod common;

/// 验证网关对未注册路由返回 404，
/// 确保通配规则不会把未知路径误当成内部错误（500）
#[tokio::test]
async fn gateway_returns_404_for_unmatched_route() {
    let server = common::test_server().await;
    let response = server.get("/gw/nonexistent").await;
    response.assert_status(StatusCode::NOT_FOUND);
    let body: serde_json::Value = response.json();
    assert_eq!(body["status"], 404);
}
