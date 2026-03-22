/// 沙箱三模式全面 E2E 测试 — 通过 HTTP 端点验证 mock / replay / proxy 的完整行为。
/// mock 模式覆盖字段类型推断、语义感知生成、enum 约束、嵌套对象、fixed_response 等场景；
/// replay 模式覆盖无录音 404、有录音精确匹配、缺少会话头 400 等场景；
/// proxy 模式覆盖 wiremock 转发和 read_only 约束；
/// 会话管理覆盖创建 → 列出 → 删除 → 验证已删除的生命周期
use api_anything_common::models::{SandboxMode, SourceType};
use api_anything_gateway::dispatcher::BackendDispatcher;
use api_anything_gateway::loader::RouteLoader;
use api_anything_gateway::router::DynamicRouter;
use api_anything_metadata::{MetadataRepo, PgMetadataRepo};
use api_anything_platform_api::build_app;
use api_anything_platform_api::state::AppState;
use axum::http::StatusCode;
use axum_test::TestServer;
use chrono::Utc;
use dashmap::DashMap;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;
use wiremock::matchers::{header, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

mod common;

fn calculator_wsdl() -> String {
    include_str!("../../generator/tests/fixtures/calculator.wsdl").to_string()
}

fn to_kebab_strip_service(s: &str) -> String {
    let mut result = String::new();
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() && i > 0 {
            result.push('-');
        }
        result.push(ch.to_lowercase().next().unwrap());
    }
    result
        .strip_suffix("-service")
        .unwrap_or(&result)
        .to_string()
}

/// 构造 SOAP Add 操作的正常响应 XML
fn soap_add_response(result: i32) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
  <soap:Body>
    <AddResponse xmlns="http://example.com/calculator">
      <result>{result}</result>
    </AddResponse>
  </soap:Body>
</soap:Envelope>"#
    )
}

/// 搭建测试环境：WSDL 生成路由 → 加载路由表 → 构建 TestServer。
/// mock_server 为 Option，proxy 模式测试传入 Some 以替换 WSDL 中的 endpoint，
/// 其他模式传 None 使用原始 example.com 地址（不会实际请求）
async fn setup(
    mock_server: Option<&MockServer>,
) -> (TestServer, Arc<PgMetadataRepo>, sqlx::PgPool, Uuid, String) {
    let pool = common::test_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let suffix = Uuid::new_v4();
    let unique_slug = format!("sbxfull{}", &suffix.to_string().replace('-', "")[..16]);

    let mut wsdl = calculator_wsdl().replace(
        r#"name="CalculatorService""#,
        &format!(r#"name="{unique_slug}Service""#),
    );
    // proxy 模式时将 endpoint 替换为 wiremock 地址
    if let Some(ms) = mock_server {
        wsdl = wsdl.replace(
            r#"location="http://example.com/calculator""#,
            &format!(r#"location="{}""#, ms.uri()),
        );
    }

    let project = repo
        .create_project(
            &format!("e2e-sbx-full-{suffix}"),
            "E2E sandbox full test",
            "test",
            SourceType::Wsdl,
        )
        .await
        .unwrap();

    api_anything_generator::pipeline::GenerationPipeline::run_wsdl(&repo, project.id, &wsdl, None)
        .await
        .unwrap();

    let router = Arc::new(DynamicRouter::new());
    let dispatchers: Arc<DashMap<Uuid, Arc<BackendDispatcher>>> = Arc::new(DashMap::new());
    RouteLoader::load(&repo, &router, &dispatchers).await.unwrap();

    let kebab_slug = to_kebab_strip_service(&format!("{unique_slug}Service"));
    let add_path = format!("/api/v1/{kebab_slug}/add");

    let repo_arc = Arc::new(repo);
    let state = AppState {
        db: pool.clone(),
        repo: repo_arc.clone(),
        router,
        dispatchers,
    };
    let server = TestServer::new(build_app(state)).unwrap();

    (server, repo_arc, pool, project.id, add_path)
}

async fn cleanup(pool: &sqlx::PgPool, project_id: Uuid) {
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project_id)
        .execute(pool)
        .await
        .unwrap();
}

// ---------------------------------------------------------------------------
// Mock 模式
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sandbox_mock_generates_integer_fields() {
    let (server, _repo, pool, project_id, add_path) = setup(None).await;

    let resp = server
        .post(&format!("/sandbox{add_path}"))
        .add_header("X-Sandbox-Mode", "mock")
        .json(&json!({"a": 3, "b": 5}))
        .await;

    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();
    // AddResponse 的 response_schema 含 result 字段（xsd:int → integer 类型）
    assert!(
        body.get("result").is_some(),
        "Mock response should contain 'result' field, got: {body}"
    );
    assert!(
        body["result"].is_number(),
        "Mock 'result' should be a number (from integer schema), got: {}",
        body["result"]
    );

    cleanup(&pool, project_id).await;
}

#[tokio::test]
async fn sandbox_mock_smart_field_email() {
    // 手动创建路由，response_schema 含 "email" 字段
    let pool = common::test_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let suffix = Uuid::new_v4();
    let project = repo
        .create_project(
            &format!("e2e-mock-email-{suffix}"),
            "test email mock",
            "test",
            SourceType::Wsdl,
        )
        .await
        .unwrap();

    let contract = repo
        .create_contract(project.id, "1.0", "test", &json!({}))
        .await
        .unwrap();

    let binding = repo
        .create_backend_binding(
            api_anything_common::models::ProtocolType::Soap,
            &json!({"url": "http://example.com/test", "soap_action": "", "operation_name": "Test", "namespace": ""}),
            5000,
        )
        .await
        .unwrap();

    let unique_path = format!("/api/v1/mock-email-{}/test", &suffix.to_string()[..8]);
    let _route = repo
        .create_route(
            contract.id,
            api_anything_common::models::HttpMethod::Post,
            &unique_path,
            &json!({"type": "object", "properties": {"input": {"type": "string"}}}),
            &json!({
                "type": "object",
                "properties": {
                    "email": {"type": "string"},
                    "name": {"type": "string"}
                }
            }),
            &json!({}),
            binding.id,
        )
        .await
        .unwrap();

    // 构建服务器
    let router = Arc::new(DynamicRouter::new());
    let dispatchers: Arc<DashMap<Uuid, Arc<BackendDispatcher>>> = Arc::new(DashMap::new());
    RouteLoader::load(&repo, &router, &dispatchers).await.unwrap();

    let state = AppState {
        db: pool.clone(),
        repo: Arc::new(PgMetadataRepo::new(pool.clone())),
        router,
        dispatchers,
    };
    let server = TestServer::new(build_app(state)).unwrap();

    let resp = server
        .post(&format!("/sandbox{unique_path}"))
        .add_header("X-Sandbox-Mode", "mock")
        .json(&json!({"input": "test"}))
        .await;

    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();
    // MockLayer 的语义推断应为 email 字段生成含 "@" 的值
    assert!(
        body["email"].as_str().unwrap_or("").contains("@"),
        "email field should contain '@', got: {}",
        body["email"]
    );

    // 清理
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project.id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn sandbox_mock_smart_field_id() {
    let pool = common::test_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let suffix = Uuid::new_v4();
    let project = repo
        .create_project(&format!("e2e-mock-id-{suffix}"), "test id mock", "test", SourceType::Wsdl)
        .await
        .unwrap();

    let contract = repo.create_contract(project.id, "1.0", "test", &json!({})).await.unwrap();
    let binding = repo
        .create_backend_binding(
            api_anything_common::models::ProtocolType::Soap,
            &json!({"url": "http://example.com/test", "soap_action": "", "operation_name": "Test", "namespace": ""}),
            5000,
        )
        .await
        .unwrap();

    let unique_path = format!("/api/v1/mock-id-{}/test", &suffix.to_string()[..8]);
    let _route = repo
        .create_route(
            contract.id,
            api_anything_common::models::HttpMethod::Post,
            &unique_path,
            &json!({"type": "object"}),
            &json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string"}
                }
            }),
            &json!({}),
            binding.id,
        )
        .await
        .unwrap();

    let router = Arc::new(DynamicRouter::new());
    let dispatchers: Arc<DashMap<Uuid, Arc<BackendDispatcher>>> = Arc::new(DashMap::new());
    RouteLoader::load(&repo, &router, &dispatchers).await.unwrap();

    let state = AppState {
        db: pool.clone(),
        repo: Arc::new(PgMetadataRepo::new(pool.clone())),
        router,
        dispatchers,
    };
    let server = TestServer::new(build_app(state)).unwrap();

    let resp = server
        .post(&format!("/sandbox{unique_path}"))
        .add_header("X-Sandbox-Mode", "mock")
        .json(&json!({}))
        .await;

    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();
    let id_val = body["id"].as_str().unwrap_or("");
    // MockLayer 为 id 字段生成 UUID 格式字符串
    assert!(
        Uuid::parse_str(id_val).is_ok(),
        "id field should be UUID format, got: {}",
        id_val
    );

    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project.id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn sandbox_mock_enum_values() {
    let pool = common::test_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let suffix = Uuid::new_v4();
    let project = repo
        .create_project(&format!("e2e-mock-enum-{suffix}"), "test", "test", SourceType::Wsdl)
        .await
        .unwrap();

    let contract = repo.create_contract(project.id, "1.0", "test", &json!({})).await.unwrap();
    let binding = repo
        .create_backend_binding(
            api_anything_common::models::ProtocolType::Soap,
            &json!({"url": "http://example.com/test", "soap_action": "", "operation_name": "Test", "namespace": ""}),
            5000,
        )
        .await
        .unwrap();

    let unique_path = format!("/api/v1/mock-enum-{}/test", &suffix.to_string()[..8]);
    let _route = repo
        .create_route(
            contract.id,
            api_anything_common::models::HttpMethod::Post,
            &unique_path,
            &json!({"type": "object"}),
            &json!({
                "type": "object",
                "properties": {
                    "priority": {"type": "string", "enum": ["active", "inactive", "pending"]}
                }
            }),
            &json!({}),
            binding.id,
        )
        .await
        .unwrap();

    let router = Arc::new(DynamicRouter::new());
    let dispatchers: Arc<DashMap<Uuid, Arc<BackendDispatcher>>> = Arc::new(DashMap::new());
    RouteLoader::load(&repo, &router, &dispatchers).await.unwrap();

    let state = AppState {
        db: pool.clone(),
        repo: Arc::new(PgMetadataRepo::new(pool.clone())),
        router,
        dispatchers,
    };
    let server = TestServer::new(build_app(state)).unwrap();

    let resp = server
        .post(&format!("/sandbox{unique_path}"))
        .add_header("X-Sandbox-Mode", "mock")
        .json(&json!({}))
        .await;

    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();
    let val = body["priority"].as_str().unwrap_or("");
    // MockLayer 在 enum 约束下应返回 enum 列表中的值
    assert!(
        ["active", "inactive", "pending"].contains(&val),
        "priority should be one of enum values, got: {}",
        val
    );

    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project.id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn sandbox_mock_array_type() {
    let pool = common::test_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let suffix = Uuid::new_v4();
    let project = repo
        .create_project(&format!("e2e-mock-arr-{suffix}"), "test", "test", SourceType::Wsdl)
        .await
        .unwrap();

    let contract = repo.create_contract(project.id, "1.0", "test", &json!({})).await.unwrap();
    let binding = repo
        .create_backend_binding(
            api_anything_common::models::ProtocolType::Soap,
            &json!({"url": "http://example.com/test", "soap_action": "", "operation_name": "Test", "namespace": ""}),
            5000,
        )
        .await
        .unwrap();

    let unique_path = format!("/api/v1/mock-arr-{}/test", &suffix.to_string()[..8]);
    let _route = repo
        .create_route(
            contract.id,
            api_anything_common::models::HttpMethod::Post,
            &unique_path,
            &json!({"type": "object"}),
            &json!({
                "type": "object",
                "properties": {
                    "items": {
                        "type": "array",
                        "items": {"type": "string"}
                    }
                }
            }),
            &json!({}),
            binding.id,
        )
        .await
        .unwrap();

    let router = Arc::new(DynamicRouter::new());
    let dispatchers: Arc<DashMap<Uuid, Arc<BackendDispatcher>>> = Arc::new(DashMap::new());
    RouteLoader::load(&repo, &router, &dispatchers).await.unwrap();

    let state = AppState {
        db: pool.clone(),
        repo: Arc::new(PgMetadataRepo::new(pool.clone())),
        router,
        dispatchers,
    };
    let server = TestServer::new(build_app(state)).unwrap();

    let resp = server
        .post(&format!("/sandbox{unique_path}"))
        .add_header("X-Sandbox-Mode", "mock")
        .json(&json!({}))
        .await;

    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();
    assert!(
        body["items"].is_array(),
        "items field should be an array, got: {}",
        body["items"]
    );
    let arr = body["items"].as_array().unwrap();
    assert!(
        !arr.is_empty(),
        "items array should have at least 1 element"
    );

    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project.id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn sandbox_mock_nested_object() {
    let pool = common::test_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let suffix = Uuid::new_v4();
    let project = repo
        .create_project(&format!("e2e-mock-nested-{suffix}"), "test", "test", SourceType::Wsdl)
        .await
        .unwrap();

    let contract = repo.create_contract(project.id, "1.0", "test", &json!({})).await.unwrap();
    let binding = repo
        .create_backend_binding(
            api_anything_common::models::ProtocolType::Soap,
            &json!({"url": "http://example.com/test", "soap_action": "", "operation_name": "Test", "namespace": ""}),
            5000,
        )
        .await
        .unwrap();

    let unique_path = format!("/api/v1/mock-nested-{}/test", &suffix.to_string()[..8]);
    let _route = repo
        .create_route(
            contract.id,
            api_anything_common::models::HttpMethod::Post,
            &unique_path,
            &json!({"type": "object"}),
            &json!({
                "type": "object",
                "properties": {
                    "user": {
                        "type": "object",
                        "properties": {
                            "name": {"type": "string"},
                            "age": {"type": "integer"}
                        }
                    }
                }
            }),
            &json!({}),
            binding.id,
        )
        .await
        .unwrap();

    let router = Arc::new(DynamicRouter::new());
    let dispatchers: Arc<DashMap<Uuid, Arc<BackendDispatcher>>> = Arc::new(DashMap::new());
    RouteLoader::load(&repo, &router, &dispatchers).await.unwrap();

    let state = AppState {
        db: pool.clone(),
        repo: Arc::new(PgMetadataRepo::new(pool.clone())),
        router,
        dispatchers,
    };
    let server = TestServer::new(build_app(state)).unwrap();

    let resp = server
        .post(&format!("/sandbox{unique_path}"))
        .add_header("X-Sandbox-Mode", "mock")
        .json(&json!({}))
        .await;

    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();
    assert!(
        body["user"].is_object(),
        "user field should be an object, got: {}",
        body["user"]
    );
    // 嵌套对象内字段也应被生成
    assert!(
        body["user"]["name"].is_string(),
        "user.name should be a string, got: {}",
        body["user"]["name"]
    );
    assert!(
        body["user"]["age"].is_number(),
        "user.age should be a number, got: {}",
        body["user"]["age"]
    );

    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project.id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn sandbox_mock_fixed_response() {
    let (server, repo, pool, project_id, add_path) = setup(None).await;

    // 创建带 fixed_response 配置的会话
    let fixed = json!({"result": 42, "source": "fixed"});
    let session = repo
        .as_ref()
        .create_sandbox_session(
            project_id,
            "test-tenant",
            SandboxMode::Mock,
            &json!({"fixed_response": fixed}),
            Utc::now() + chrono::Duration::hours(1),
        )
        .await
        .unwrap();

    let resp = server
        .post(&format!("/sandbox{add_path}"))
        .add_header("X-Sandbox-Mode", "mock")
        .add_header("X-Sandbox-Session", &session.id.to_string())
        .json(&json!({"a": 1, "b": 2}))
        .await;

    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();
    assert_eq!(body["result"], 42, "Should return fixed_response, got: {body}");
    assert_eq!(body["source"], "fixed");

    cleanup(&pool, project_id).await;
}

#[tokio::test]
async fn sandbox_mock_default_mode_no_header() {
    // 不传 X-Sandbox-Mode 时默认 mock 模式
    let (server, _repo, pool, project_id, add_path) = setup(None).await;

    let resp = server
        .post(&format!("/sandbox{add_path}"))
        .json(&json!({"a": 1, "b": 2}))
        .await;

    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();
    assert!(
        body.get("result").is_some(),
        "Default mode (mock) should return result field, got: {body}"
    );

    cleanup(&pool, project_id).await;
}

// ---------------------------------------------------------------------------
// Replay 模式
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sandbox_replay_no_recording_returns_404() {
    let (server, repo, pool, project_id, add_path) = setup(None).await;

    // 创建 replay 会话但不录制任何交互
    let session = repo
        .as_ref()
        .create_sandbox_session(
            project_id,
            "test-tenant",
            SandboxMode::Replay,
            &json!({}),
            Utc::now() + chrono::Duration::hours(1),
        )
        .await
        .unwrap();

    let resp = server
        .post(&format!("/sandbox{add_path}"))
        .add_header("X-Sandbox-Mode", "replay")
        .add_header("X-Sandbox-Session", &session.id.to_string())
        .json(&json!({"a": 3, "b": 5}))
        .await;

    // 无录音时返回 404
    resp.assert_status(StatusCode::NOT_FOUND);

    cleanup(&pool, project_id).await;
}

#[tokio::test]
async fn sandbox_replay_matches_after_recording() {
    let (server, repo, pool, project_id, add_path) = setup(None).await;

    // 找到路由 id
    let routes = repo.as_ref().list_active_routes_with_bindings().await.unwrap();
    let target_route = routes
        .iter()
        .find(|r| r.path == add_path)
        .expect("Should find the add route");
    let route_id = target_route.route_id;

    // 创建 replay 会话
    let session = repo
        .as_ref()
        .create_sandbox_session(
            project_id,
            "test-tenant",
            SandboxMode::Replay,
            &json!({}),
            Utc::now() + chrono::Duration::hours(1),
        )
        .await
        .unwrap();

    // 手动插入录音记录
    let recorded_request = json!({"a": 7, "b": 3});
    let recorded_response = json!({"result": 10});
    repo.as_ref()
        .record_interaction(session.id, route_id, &recorded_request, &recorded_response, 50)
        .await
        .unwrap();

    // replay 发送匹配请求
    let resp = server
        .post(&format!("/sandbox{add_path}"))
        .add_header("X-Sandbox-Mode", "replay")
        .add_header("X-Sandbox-Session", &session.id.to_string())
        .json(&recorded_request)
        .await;

    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();
    assert_eq!(
        body["result"], 10,
        "Replay should return recorded response, got: {body}"
    );

    cleanup(&pool, project_id).await;
}

#[tokio::test]
async fn sandbox_replay_requires_session_header() {
    let (server, _repo, pool, project_id, add_path) = setup(None).await;

    // replay 模式不提供 X-Sandbox-Session → 400
    let resp = server
        .post(&format!("/sandbox{add_path}"))
        .add_header("X-Sandbox-Mode", "replay")
        .json(&json!({"a": 1, "b": 2}))
        .await;

    resp.assert_status(StatusCode::BAD_REQUEST);

    cleanup(&pool, project_id).await;
}

// ---------------------------------------------------------------------------
// Proxy 模式
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sandbox_proxy_forwards_to_backend() {
    // 启动 wiremock 模拟 SOAP 后端
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("Content-Type", "text/xml; charset=utf-8"))
        .respond_with(ResponseTemplate::new(200).set_body_string(soap_add_response(15)))
        .expect(1..)
        .mount(&mock_server)
        .await;

    let (server, repo, pool, project_id, add_path) = setup(Some(&mock_server)).await;

    // 创建 proxy 会话
    let session = repo
        .as_ref()
        .create_sandbox_session(
            project_id,
            "test-tenant",
            SandboxMode::Proxy,
            &json!({}),
            Utc::now() + chrono::Duration::hours(1),
        )
        .await
        .unwrap();

    let resp = server
        .post(&format!("/sandbox{add_path}"))
        .add_header("X-Sandbox-Mode", "proxy")
        .add_header("X-Sandbox-Session", &session.id.to_string())
        .json(&json!({"a": 7, "b": 8}))
        .await;

    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();
    // SoapXmlParser 将 XML 文本节点解析为 string
    assert_eq!(
        body["result"], "15",
        "Proxy should forward to wiremock and return result, got: {body}"
    );

    cleanup(&pool, project_id).await;
}

#[tokio::test]
async fn sandbox_proxy_read_only_blocks_post() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(soap_add_response(0)))
        .mount(&mock_server)
        .await;

    let (server, repo, pool, project_id, add_path) = setup(Some(&mock_server)).await;

    // 创建 read_only proxy 会话
    let session = repo
        .as_ref()
        .create_sandbox_session(
            project_id,
            "test-tenant",
            SandboxMode::Proxy,
            &json!({"read_only": true}),
            Utc::now() + chrono::Duration::hours(1),
        )
        .await
        .unwrap();

    // POST 请求在 read_only 模式下应被拒绝
    let resp = server
        .post(&format!("/sandbox{add_path}"))
        .add_header("X-Sandbox-Mode", "proxy")
        .add_header("X-Sandbox-Session", &session.id.to_string())
        .json(&json!({"a": 1, "b": 2}))
        .await;

    resp.assert_status(StatusCode::BAD_REQUEST);

    cleanup(&pool, project_id).await;
}

#[tokio::test]
async fn sandbox_proxy_requires_session_header() {
    let mock_server = MockServer::start().await;
    let (server, _repo, pool, project_id, add_path) = setup(Some(&mock_server)).await;

    // proxy 模式不提供 session header → 400
    let resp = server
        .post(&format!("/sandbox{add_path}"))
        .add_header("X-Sandbox-Mode", "proxy")
        .json(&json!({"a": 1, "b": 2}))
        .await;

    resp.assert_status(StatusCode::BAD_REQUEST);

    cleanup(&pool, project_id).await;
}

// ---------------------------------------------------------------------------
// 会话管理
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sandbox_session_lifecycle() {
    let (server, _repo, pool, project_id, _add_path) = setup(None).await;

    // 1. 创建会话
    let create_resp = server
        .post(&format!(
            "/api/v1/projects/{project_id}/sandbox-sessions"
        ))
        .json(&json!({
            "tenant_id": "lifecycle-tenant",
            "mode": "mock",
            "config": {},
            "expires_in_hours": 1
        }))
        .await;
    create_resp.assert_status(StatusCode::CREATED);
    let session: serde_json::Value = create_resp.json();
    let session_id = session["id"].as_str().unwrap();

    // 2. 列出会话
    let list_resp = server
        .get(&format!(
            "/api/v1/projects/{project_id}/sandbox-sessions"
        ))
        .await;
    list_resp.assert_status(StatusCode::OK);
    let sessions: Vec<serde_json::Value> = list_resp.json();
    assert!(
        sessions.iter().any(|s| s["id"].as_str() == Some(session_id)),
        "Created session should appear in list"
    );

    // 3. 删除会话
    let delete_resp = server
        .delete(&format!("/api/v1/sandbox-sessions/{session_id}"))
        .await;
    delete_resp.assert_status(StatusCode::NO_CONTENT);

    // 4. 验证已删除 — 再列出，不应包含已删除的会话
    let list_resp2 = server
        .get(&format!(
            "/api/v1/projects/{project_id}/sandbox-sessions"
        ))
        .await;
    list_resp2.assert_status(StatusCode::OK);
    let sessions2: Vec<serde_json::Value> = list_resp2.json();
    assert!(
        !sessions2.iter().any(|s| s["id"].as_str() == Some(session_id)),
        "Deleted session should not appear in list"
    );

    cleanup(&pool, project_id).await;
}

#[tokio::test]
async fn sandbox_session_expired_still_usable() {
    // 过期时间为过去的会话仍可使用（过期清理是后台任务）
    let (server, repo, pool, project_id, add_path) = setup(None).await;

    let session = repo
        .as_ref()
        .create_sandbox_session(
            project_id,
            "test-tenant",
            SandboxMode::Mock,
            &json!({"fixed_response": {"result": 99}}),
            // 过期时间设在过去
            Utc::now() - chrono::Duration::hours(1),
        )
        .await
        .unwrap();

    // 即使会话已"过期"，mock 模式仍能正常使用
    let resp = server
        .post(&format!("/sandbox{add_path}"))
        .add_header("X-Sandbox-Mode", "mock")
        .add_header("X-Sandbox-Session", &session.id.to_string())
        .json(&json!({"a": 1, "b": 2}))
        .await;

    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();
    assert_eq!(body["result"], 99);

    cleanup(&pool, project_id).await;
}
