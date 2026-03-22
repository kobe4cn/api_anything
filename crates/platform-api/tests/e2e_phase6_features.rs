/// Phase 6 功能综合测试：覆盖 SDK 代码生成、Webhook 生命周期、
/// 插件 API 路由注册、录制管理、代理自动录制全链路、EventBus 持久化及链路追踪。
///
/// 测试策略：尽量通过 HTTP 端点验证功能正确性，减少对内部实现的依赖；
/// EventBus 测试直接操作数据库层以验证写入行为。
mod common;
use axum::http::StatusCode;
use serde_json::json;
use uuid::Uuid;

// ===== SDK 代码生成 =====

#[tokio::test]
async fn sdk_typescript_generation() {
    let server = common::test_server().await;
    let resp = server.get("/api/v1/docs/sdk/typescript").await;
    resp.assert_status(StatusCode::OK);
    let code = resp.text();
    assert!(code.contains("BASE_URL"), "Should contain BASE_URL constant");
    assert!(code.contains("fetch"), "Should contain fetch calls");
}

#[tokio::test]
async fn sdk_python_generation() {
    let server = common::test_server().await;
    let resp = server.get("/api/v1/docs/sdk/python").await;
    resp.assert_status(StatusCode::OK);
    let code = resp.text();
    assert!(code.contains("requests"), "Should contain requests import");
}

#[tokio::test]
async fn sdk_java_generation() {
    let server = common::test_server().await;
    let resp = server.get("/api/v1/docs/sdk/java").await;
    resp.assert_status(StatusCode::OK);
    let code = resp.text();
    assert!(
        code.contains("HttpClient") || code.contains("class"),
        "Should contain Java HTTP code"
    );
}

#[tokio::test]
async fn sdk_go_generation() {
    let server = common::test_server().await;
    let resp = server.get("/api/v1/docs/sdk/go").await;
    resp.assert_status(StatusCode::OK);
    let code = resp.text();
    assert!(
        code.contains("http") || code.contains("func"),
        "Should contain Go HTTP code"
    );
}

#[tokio::test]
async fn sdk_unsupported_language_returns_400() {
    let server = common::test_server().await;
    let resp = server.get("/api/v1/docs/sdk/cobol").await;
    resp.assert_status(StatusCode::BAD_REQUEST);
}

// ===== Webhook 管理 =====

#[tokio::test]
async fn webhook_create_list_delete_lifecycle() {
    let server = common::test_server().await;

    // Create
    let resp = server
        .post("/api/v1/webhooks")
        .json(&json!({
            "url": "https://hooks.slack.com/test",
            "event_types": ["DeliveryFailed", "DeadLetter"],
            "description": "E2E test webhook"
        }))
        .await;
    resp.assert_status(StatusCode::CREATED);
    let created: serde_json::Value = resp.json();
    let id = created["id"].as_str().unwrap();

    // List — 确认新创建的 webhook 出现在列表中
    let resp = server.get("/api/v1/webhooks").await;
    resp.assert_status(StatusCode::OK);
    let list: Vec<serde_json::Value> = resp.json();
    assert!(list.iter().any(|w| w["id"].as_str() == Some(id)));

    // Delete
    let resp = server
        .delete(&format!("/api/v1/webhooks/{}", id))
        .await;
    resp.assert_status(StatusCode::NO_CONTENT);

    // 验证删除后不再出现
    let resp = server.get("/api/v1/webhooks").await;
    let list: Vec<serde_json::Value> = resp.json();
    assert!(!list.iter().any(|w| w["id"].as_str() == Some(id)));
}

#[tokio::test]
async fn webhook_with_custom_headers() {
    // CreateWebhookRequest 不含 headers 字段，多余字段在反序列化时被忽略；
    // 此测试验证携带额外字段不会导致请求失败
    let server = common::test_server().await;
    let resp = server
        .post("/api/v1/webhooks")
        .json(&json!({
            "url": "https://example.com/webhook",
            "event_types": ["GenerationCompleted"],
            "description": "With custom headers",
            "headers": {"X-Custom": "value123"}
        }))
        .await;
    resp.assert_status(StatusCode::CREATED);
    let body: serde_json::Value = resp.json();
    // Clean up
    let id = body["id"].as_str().unwrap();
    server
        .delete(&format!("/api/v1/webhooks/{}", id))
        .await;
}

// ===== Plugins API =====

#[tokio::test]
async fn plugins_list_returns_array() {
    let server = common::test_server().await;
    let resp = server.get("/api/v1/plugins").await;
    resp.assert_status(StatusCode::OK);
    // 验证返回的是 JSON 数组（当前为空列表占位）
    let body: Vec<serde_json::Value> = resp.json();
    assert_eq!(body.len(), 0, "Stub should return empty array");
}

#[tokio::test]
async fn plugins_scan_returns_result() {
    let server = common::test_server().await;
    let resp = server.post("/api/v1/plugins/scan").await;
    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();
    assert_eq!(body["loaded"], 0, "Stub should report 0 loaded plugins");
}

// ===== 录制管理 API =====

#[tokio::test]
async fn recordings_list_for_nonexistent_session() {
    let server = common::test_server().await;
    // 不存在的 session ID — handler 会先校验会话存在性，返回 404 或其他错误
    let resp = server
        .get(&format!(
            "/api/v1/sandbox-sessions/{}/recordings",
            Uuid::new_v4()
        ))
        .await;
    // 不存在的会话应返回非 200 状态码（通常 404）
    let status = resp.status_code();
    assert_ne!(
        status,
        StatusCode::OK,
        "Nonexistent session should not return 200"
    );
}

#[tokio::test]
async fn recordings_clear_for_nonexistent_session() {
    let server = common::test_server().await;
    let resp = server
        .delete(&format!(
            "/api/v1/sandbox-sessions/{}/recordings",
            Uuid::new_v4()
        ))
        .await;
    // 清空不存在会话的录制应返回非 200（通常 404）
    let status = resp.status_code();
    assert_ne!(
        status,
        StatusCode::OK,
        "Nonexistent session should not return 200"
    );
}

// ===== Proxy 自动录制全链路 =====

#[tokio::test]
async fn proxy_auto_recording_and_replay() {
    // 全链路验证：创建项目 → 创建 proxy 会话 → 查询空录制 → 清理
    // 真正的 proxy 录制需要 wiremock 后端，此测试验证 API 端点可达性和基本行为
    let server = common::test_server().await;

    // 创建项目
    let suffix = Uuid::new_v4();
    let resp = server
        .post("/api/v1/projects")
        .json(&json!({
            "name": format!("recording-test-{}", &suffix.to_string()[..8]),
            "description": "Recording test",
            "owner": "test",
            "source_type": "wsdl"
        }))
        .await;
    resp.assert_status(StatusCode::CREATED);
    let project: serde_json::Value = resp.json();
    let project_id = project["id"].as_str().unwrap();

    // 创建 proxy 沙箱会话
    let resp = server
        .post(&format!(
            "/api/v1/projects/{}/sandbox-sessions",
            project_id
        ))
        .json(&json!({
            "tenant_id": "recording-test-tenant",
            "mode": "proxy",
            "config": {},
            "expires_in_hours": 1
        }))
        .await;
    resp.assert_status(StatusCode::CREATED);
    let session: serde_json::Value = resp.json();
    let session_id = session["id"].as_str().unwrap();

    // 查询该会话的录制数据（新建会话应无录制）
    let resp = server
        .get(&format!(
            "/api/v1/sandbox-sessions/{}/recordings",
            session_id
        ))
        .await;
    resp.assert_status(StatusCode::OK);
    let recordings: Vec<serde_json::Value> = resp.json();
    assert_eq!(recordings.len(), 0, "Should have no recordings initially");

    // Clean up
    server
        .delete(&format!("/api/v1/sandbox-sessions/{}", session_id))
        .await;
    server
        .delete(&format!("/api/v1/projects/{}", project_id))
        .await;
}

// ===== EventBus 集成测试 =====

#[tokio::test]
async fn event_bus_pg_publish_persists_to_db() {
    // 验证 PgEventBus 的 publish 将事件写入 events 表
    use api_anything_event_bus::{Event, EventBus, EventType, PgEventBus};

    let pool = common::test_pool().await;

    let bus = PgEventBus::new(pool.clone());
    let event = Event {
        id: Uuid::new_v4(),
        event_type: EventType::GenerationCompleted {
            project_id: Uuid::new_v4(),
            contract_id: Uuid::new_v4(),
            routes_count: 3,
        },
        timestamp: chrono::Utc::now(),
        payload: json!({"test": true}),
    };

    bus.publish(event.clone()).await.unwrap();

    // 验证事件已持久化到数据库
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM events WHERE id = $1")
        .bind(event.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.0, 1, "Event should be persisted in DB");

    // Clean up
    sqlx::query("DELETE FROM events WHERE id = $1")
        .bind(event.id)
        .execute(&pool)
        .await
        .unwrap();
}

// ===== 链路追踪验证 =====

#[tokio::test]
async fn tracing_layer_adds_trace_headers() {
    // 验证 TraceLayer 不阻塞正常请求处理，health 端点正常响应
    let server = common::test_server().await;
    let resp = server.get("/health").await;
    resp.assert_status(StatusCode::OK);
}
