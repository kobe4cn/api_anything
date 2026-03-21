use axum::http::StatusCode;
use axum_test::TestServer;
use serde_json::json;

mod common;

#[tokio::test]
async fn create_project_returns_201() {
    let server = common::test_server().await;
    // 使用 UUID 后缀避免 name 列 UNIQUE 约束在重复运行时冲突
    let name = format!("test-soap-{}", uuid::Uuid::new_v4());
    let response = server
        .post("/api/v1/projects")
        .json(&json!({
            "name": name,
            "description": "Test SOAP service",
            "owner": "team-test",
            "source_type": "wsdl"
        }))
        .await;
    response.assert_status(StatusCode::CREATED);
    let body: serde_json::Value = response.json();
    assert_eq!(body["source_type"], "wsdl");
    assert!(body["id"].is_string());
}

#[tokio::test]
async fn get_project_returns_404_for_unknown_id() {
    let server = common::test_server().await;
    let response = server
        .get("/api/v1/projects/00000000-0000-0000-0000-000000000000")
        .await;
    response.assert_status(StatusCode::NOT_FOUND);
    let body: serde_json::Value = response.json();
    assert_eq!(body["status"], 404);
}

#[tokio::test]
async fn list_projects_returns_array() {
    let server = common::test_server().await;
    let response = server.get("/api/v1/projects").await;
    response.assert_status(StatusCode::OK);
    let body: serde_json::Value = response.json();
    assert!(body.is_array());
}

#[tokio::test]
async fn create_then_get_project() {
    let server = common::test_server().await;
    let name = format!("roundtrip-{}", uuid::Uuid::new_v4());
    let create_resp = server
        .post("/api/v1/projects")
        .json(&json!({
            "name": name,
            "description": "Roundtrip test",
            "owner": "team-test",
            "source_type": "cli"
        }))
        .await;
    let created: serde_json::Value = create_resp.json();
    let id = created["id"].as_str().unwrap();

    let get_resp = server.get(&format!("/api/v1/projects/{id}")).await;
    get_resp.assert_status(StatusCode::OK);
    let fetched: serde_json::Value = get_resp.json();
    assert_eq!(fetched["source_type"], "cli");
}

#[tokio::test]
async fn delete_project_returns_204() {
    let server = common::test_server().await;
    let name = format!("to-delete-{}", uuid::Uuid::new_v4());
    let create_resp = server
        .post("/api/v1/projects")
        .json(&json!({
            "name": name,
            "description": "Will be deleted",
            "owner": "team-test",
            "source_type": "ssh"
        }))
        .await;
    let created: serde_json::Value = create_resp.json();
    let id = created["id"].as_str().unwrap();

    let del_resp = server.delete(&format!("/api/v1/projects/{id}")).await;
    del_resp.assert_status(StatusCode::NO_CONTENT);

    // 删除后再次 GET 应返回 404，确认记录已从数据库移除
    let get_resp = server.get(&format!("/api/v1/projects/{id}")).await;
    get_resp.assert_status(StatusCode::NOT_FOUND);
}
