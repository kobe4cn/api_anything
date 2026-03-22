use axum::http::StatusCode;
use axum_test::TestServer;
use serde_json::json;

mod common;

#[tokio::test]
async fn create_sandbox_session_returns_201() {
    let server = common::test_server().await;
    // 先创建 project，sandbox_sessions 通过外键依赖 projects 表
    let project_name = format!("sandbox-proj-{}", uuid::Uuid::new_v4());
    let proj_resp = server
        .post("/api/v1/projects")
        .json(&json!({
            "name": project_name,
            "description": "Sandbox test project",
            "owner": "team-test",
            "source_type": "wsdl"
        }))
        .await;
    proj_resp.assert_status(StatusCode::CREATED);
    let proj: serde_json::Value = proj_resp.json();
    let project_id = proj["id"].as_str().unwrap();

    let resp = server
        .post(&format!("/api/v1/projects/{project_id}/sandbox-sessions"))
        .json(&json!({
            "tenant_id": "team-frontend",
            "mode": "mock",
            "config": {},
            "expires_in_hours": 24
        }))
        .await;
    resp.assert_status(StatusCode::CREATED);
    let body: serde_json::Value = resp.json();
    assert!(body["id"].is_string());
    assert_eq!(body["mode"], "mock");
    assert_eq!(body["tenant_id"], "team-frontend");
}

#[tokio::test]
async fn list_sessions_returns_array() {
    let server = common::test_server().await;
    let project_name = format!("sandbox-list-proj-{}", uuid::Uuid::new_v4());
    let proj_resp = server
        .post("/api/v1/projects")
        .json(&json!({
            "name": project_name,
            "description": "Sandbox list test project",
            "owner": "team-test",
            "source_type": "cli"
        }))
        .await;
    let proj: serde_json::Value = proj_resp.json();
    let project_id = proj["id"].as_str().unwrap();

    // 创建一个会话以确保列表不为空
    server
        .post(&format!("/api/v1/projects/{project_id}/sandbox-sessions"))
        .json(&json!({
            "tenant_id": "team-backend",
            "mode": "replay",
            "config": {"replay_strict": true},
            "expires_in_hours": 8
        }))
        .await;

    let resp = server
        .get(&format!("/api/v1/projects/{project_id}/sandbox-sessions"))
        .await;
    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();
    assert!(body.is_array());
    // 列表至少包含刚创建的会话
    assert!(body.as_array().unwrap().len() >= 1);
}

#[tokio::test]
async fn delete_session_returns_204() {
    let server = common::test_server().await;
    let project_name = format!("sandbox-del-proj-{}", uuid::Uuid::new_v4());
    let proj_resp = server
        .post("/api/v1/projects")
        .json(&json!({
            "name": project_name,
            "description": "Sandbox delete test project",
            "owner": "team-test",
            "source_type": "ssh"
        }))
        .await;
    let proj: serde_json::Value = proj_resp.json();
    let project_id = proj["id"].as_str().unwrap();

    let create_resp = server
        .post(&format!("/api/v1/projects/{project_id}/sandbox-sessions"))
        .json(&json!({
            "tenant_id": "team-ops",
            "mode": "proxy",
            "config": {},
            "expires_in_hours": 1
        }))
        .await;
    create_resp.assert_status(StatusCode::CREATED);
    let session: serde_json::Value = create_resp.json();
    let session_id = session["id"].as_str().unwrap();

    let del_resp = server
        .delete(&format!("/api/v1/sandbox-sessions/{session_id}"))
        .await;
    del_resp.assert_status(StatusCode::NO_CONTENT);

    // 删除后继续删除同一 ID 应返回 404，确认幂等删除不会静默成功
    let del_again = server
        .delete(&format!("/api/v1/sandbox-sessions/{session_id}"))
        .await;
    del_again.assert_status(StatusCode::NOT_FOUND);
}
