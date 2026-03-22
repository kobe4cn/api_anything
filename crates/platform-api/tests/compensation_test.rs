use axum::http::StatusCode;
use serde_json::Value;

mod common;

#[tokio::test]
async fn list_dead_letters_returns_array() {
    let server = common::test_server().await;
    let resp = server.get("/api/v1/compensation/dead-letters").await;
    resp.assert_status(StatusCode::OK);
    let body: Value = resp.json();
    assert!(body.is_array());
}

#[tokio::test]
async fn get_nonexistent_delivery_record_returns_404() {
    let server = common::test_server().await;
    let resp = server
        .get("/api/v1/compensation/delivery-records/00000000-0000-0000-0000-000000000000")
        .await;
    resp.assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn retry_nonexistent_dead_letter_returns_404() {
    let server = common::test_server().await;
    let resp = server
        .post("/api/v1/compensation/dead-letters/00000000-0000-0000-0000-000000000000/retry")
        .await;
    resp.assert_status(StatusCode::NOT_FOUND);
}
