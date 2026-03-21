use axum::http::StatusCode;
use axum_test::TestServer;

mod common;

#[tokio::test]
async fn health_check_returns_ok() {
    let server = common::test_server().await;
    let response = server.get("/health").await;
    response.assert_status(StatusCode::OK);
    let body: serde_json::Value = response.json();
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn health_ready_returns_ok_when_db_connected() {
    let server = common::test_server().await;
    let response = server.get("/health/ready").await;
    response.assert_status(StatusCode::OK);
}
