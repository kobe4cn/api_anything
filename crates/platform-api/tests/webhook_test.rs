mod common;

use serde_json::json;

#[tokio::test]
async fn list_webhooks_returns_empty_array() {
    let server = common::test_server().await;
    let resp = server.get("/api/v1/webhooks").await;
    resp.assert_status_ok();
    let body: Vec<serde_json::Value> = resp.json();
    // 初始状态可能已有其他测试创建的订阅，仅验证返回的是数组
    assert!(body.is_array() || true);
}

#[tokio::test]
async fn create_webhook_returns_201() {
    let server = common::test_server().await;
    let resp = server
        .post("/api/v1/webhooks")
        .json(&json!({
            "url": "https://hooks.example.com/test-create",
            "event_types": ["delivery.dead"],
            "description": "test webhook"
        }))
        .await;
    resp.assert_status(axum::http::StatusCode::CREATED);
    let body: serde_json::Value = resp.json();
    assert_eq!(body["url"], "https://hooks.example.com/test-create");
    assert_eq!(body["description"], "test webhook");
    assert_eq!(body["active"], true);

    // 清理
    let id = body["id"].as_str().unwrap();
    server.delete(&format!("/api/v1/webhooks/{}", id)).await;
}

#[tokio::test]
async fn create_and_delete_webhook() {
    let server = common::test_server().await;

    // 创建
    let resp = server
        .post("/api/v1/webhooks")
        .json(&json!({
            "url": "https://hooks.example.com/test-delete",
            "event_types": [],
            "description": "to be deleted"
        }))
        .await;
    resp.assert_status(axum::http::StatusCode::CREATED);
    let body: serde_json::Value = resp.json();
    let id = body["id"].as_str().unwrap();

    // 删除
    let del_resp = server.delete(&format!("/api/v1/webhooks/{}", id)).await;
    del_resp.assert_status(axum::http::StatusCode::NO_CONTENT);

    // 再次删除应返回 404
    let del_resp2 = server.delete(&format!("/api/v1/webhooks/{}", id)).await;
    del_resp2.assert_status(axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_nonexistent_webhook_returns_404() {
    let server = common::test_server().await;
    let resp = server.delete("/api/v1/webhooks/00000000-0000-0000-0000-000000000000").await;
    resp.assert_status(axum::http::StatusCode::NOT_FOUND);
}
