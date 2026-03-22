/// E2E 集成测试：验证沙箱 handler 的 mock / replay 三条路径。
/// 使用与 e2e_soap_proxy_test 相同的 WSDL 生成 + 路由加载流程搭建环境；
/// 所有测试均使用唯一 slug 路径，避免与其他测试的路由产生竞争
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

mod common;

/// 读取 calculator.wsdl fixture，与 e2e_soap_proxy_test 使用同一份资产
fn calculator_wsdl() -> String {
    include_str!("../../generator/tests/fixtures/calculator.wsdl").to_string()
}

/// 复现 WsdlMapper::to_kebab_case 并去掉 "-service" 后缀，预测生成的路由路径
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

/// 搭建测试环境：生成路由 → 加载路由表 → 构建 TestServer；
/// 返回 (server, repo_arc, pool, project_id, add_path) 供各测试复用；
/// 每次调用都使用新 UUID slug，确保路由路径唯一、不与其他测试冲突
async fn setup() -> (TestServer, Arc<PgMetadataRepo>, sqlx::PgPool, Uuid, String) {
    let pool = common::test_pool().await;

    // 先创建裸 repo 执行流水线操作，避免 Arc<T> 不满足 impl MetadataRepo 约束的问题；
    // Arc<PgMetadataRepo> 用于 AppState，裸引用用于 run_wsdl / RouteLoader 等需要 &impl MetadataRepo 的函数
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let suffix = Uuid::new_v4();
    // UUID 连字符去掉后取前 16 位十六进制，适合嵌入 XML name 属性且保证唯一性
    let unique_slug = format!("sbxsvc{}", &suffix.to_string().replace('-', "")[..16]);

    let wsdl = calculator_wsdl().replace(
        r#"name="CalculatorService""#,
        &format!(r#"name="{unique_slug}Service""#),
    );

    let project = repo
        .create_project(
            &format!("e2e-sandbox-{suffix}"),
            "E2E sandbox test",
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
    // 不带 /sandbox 前缀，调用方在发请求时加上
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

/// 清理测试写入的数据；外键级联删除 contracts → routes，backend_bindings 成为孤立记录但不影响隔离
async fn cleanup(pool: &sqlx::PgPool, project_id: Uuid) {
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project_id)
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn sandbox_mock_returns_data_matching_schema() {
    let (server, _repo, pool, project_id, add_path) = setup().await;

    // mock 模式：无需会话，直接从 response_schema 生成模拟数据
    let resp = server
        .post(&format!("/sandbox{add_path}"))
        .add_header("X-Sandbox-Mode", "mock")
        .json(&json!({"a": 3, "b": 5}))
        .await;

    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();
    // AddResponse 的 response_schema 含 result 字段（xsd:int → integer 类型），
    // mock 生成器应产出数字类型的值
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
async fn sandbox_mock_default_mode_when_no_header() {
    // 不传 X-Sandbox-Mode 时默认使用 mock 模式
    let (server, _repo, pool, project_id, add_path) = setup().await;

    let resp = server
        .post(&format!("/sandbox{add_path}"))
        .json(&json!({"a": 1, "b": 2}))
        .await;

    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();
    assert!(
        body.get("result").is_some(),
        "Default mock response should contain 'result' field, got: {body}"
    );

    cleanup(&pool, project_id).await;
}

#[tokio::test]
async fn sandbox_replay_returns_404_without_recordings() {
    let (server, repo, pool, project_id, add_path) = setup().await;

    // 创建 replay 模式会话；录音集为空时 ReplayLayer 应返回 404 而非静默返回空响应
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

    // 无录音时期望 404，而非 500 或空响应
    resp.assert_status(StatusCode::NOT_FOUND);

    cleanup(&pool, project_id).await;
}

#[tokio::test]
async fn sandbox_replay_returns_recorded_response() {
    let (server, repo, pool, project_id, add_path) = setup().await;

    // 直接查路由表找出 route_id，比通过 handler 间接推断更可靠
    let routes = repo.as_ref().list_active_routes_with_bindings().await.unwrap();
    // 找出路径包含 add_path 的路由（add_path 不含 /sandbox 前缀，路由表路径也不含）
    let target_route = routes
        .iter()
        .find(|r| r.path == add_path)
        .expect("Should find the add route in route table");
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

    // 预先写入一条录音记录，模拟之前代理时录制的结果
    let recorded_request = json!({"a": 3, "b": 5});
    let recorded_response = json!({"result": 8});
    repo.as_ref()
        .record_interaction(
            session.id,
            route_id,
            &recorded_request,
            &recorded_response,
            100,
        )
        .await
        .unwrap();

    // replay 应精确匹配 request 并返回对应录音
    let resp = server
        .post(&format!("/sandbox{add_path}"))
        .add_header("X-Sandbox-Mode", "replay")
        .add_header("X-Sandbox-Session", &session.id.to_string())
        .json(&recorded_request)
        .await;

    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();
    assert_eq!(
        body["result"], 8,
        "Replay should return the recorded response, got: {body}"
    );

    cleanup(&pool, project_id).await;
}

#[tokio::test]
async fn sandbox_invalid_mode_returns_400() {
    let (server, _repo, pool, project_id, add_path) = setup().await;

    let resp = server
        .post(&format!("/sandbox{add_path}"))
        .add_header("X-Sandbox-Mode", "invalid-mode")
        .json(&json!({"a": 1, "b": 2}))
        .await;

    resp.assert_status(StatusCode::BAD_REQUEST);

    cleanup(&pool, project_id).await;
}

#[tokio::test]
async fn sandbox_replay_without_session_returns_400() {
    let (server, _repo, pool, project_id, add_path) = setup().await;

    // replay 模式缺少 X-Sandbox-Session 时应返回 400，而非 500
    let resp = server
        .post(&format!("/sandbox{add_path}"))
        .add_header("X-Sandbox-Mode", "replay")
        .json(&json!({"a": 1, "b": 2}))
        .await;

    resp.assert_status(StatusCode::BAD_REQUEST);

    cleanup(&pool, project_id).await;
}

#[tokio::test]
async fn sandbox_unknown_route_returns_404() {
    let (server, _repo, pool, project_id, _add_path) = setup().await;

    // 请求一个不存在的路径，路由匹配失败应返回 404
    let resp = server
        .post("/sandbox/api/v1/nonexistent/operation")
        .add_header("X-Sandbox-Mode", "mock")
        .json(&json!({}))
        .await;

    resp.assert_status(StatusCode::NOT_FOUND);

    cleanup(&pool, project_id).await;
}

#[tokio::test]
async fn sandbox_mock_with_fixed_response_config() {
    let (server, repo, pool, project_id, add_path) = setup().await;

    // 创建带 fixed_response 配置的会话，mock 层应优先返回固定响应而非 schema 生成的数据
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
    assert_eq!(
        body["result"], 42,
        "Should return fixed_response from session config, got: {body}"
    );
    assert_eq!(
        body["source"], "fixed",
        "Should return fixed_response from session config, got: {body}"
    );

    cleanup(&pool, project_id).await;
}
