/// 文档服务全面测试：覆盖 OpenAPI 3.0 规范结构、Swagger UI、Agent Prompt 格式、
/// 以及路由变更后文档动态更新等场景。
///
/// 与 docs_test.rs 中的基础测试不同，这些测试更深入地验证了文档内容的正确性，
/// 包括 schema 结构、错误码定义、Markdown 格式等细节。
use api_anything_common::models::SourceType;
use api_anything_gateway::dispatcher::BackendDispatcher;
use api_anything_gateway::loader::RouteLoader;
use api_anything_gateway::router::DynamicRouter;
use api_anything_metadata::{MetadataRepo, PgMetadataRepo};
use api_anything_platform_api::build_app;
use api_anything_platform_api::state::AppState;
use axum::http::StatusCode;
use axum_test::TestServer;
use dashmap::DashMap;
use std::sync::Arc;
use uuid::Uuid;

mod common;

// ── 辅助函数 ────────────────────────────────────────────────────────────

/// 创建带有已生成路由的测试服务器；返回 (TestServer, project_id, pool)
/// 用于需要路由数据的测试场景，避免在每个测试中重复搭建流水线
async fn server_with_routes() -> (TestServer, Uuid, sqlx::PgPool) {
    let pool = common::test_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let suffix = Uuid::new_v4();
    let project = repo
        .create_project(
            &format!("docs-comp-{suffix}"),
            "docs comprehensive test",
            "test",
            SourceType::Wsdl,
        )
        .await
        .unwrap();

    // 使用 UUID 前缀替换服务名，确保测试间路由路径不冲突
    let wsdl = include_str!("../../generator/tests/fixtures/calculator.wsdl").replace(
        "CalculatorService",
        &format!("DocsComp{}Service", &suffix.to_string()[..8]),
    );
    api_anything_generator::pipeline::GenerationPipeline::run_wsdl(&repo, project.id, &wsdl, None)
        .await
        .unwrap();

    let router = Arc::new(DynamicRouter::new());
    let dispatchers: Arc<DashMap<Uuid, Arc<BackendDispatcher>>> = Arc::new(DashMap::new());
    RouteLoader::load(&repo, &router, &dispatchers)
        .await
        .unwrap();

    let state = AppState {
        db: pool.clone(),
        repo: Arc::new(PgMetadataRepo::new(pool.clone())),
        router,
        dispatchers,
    };
    let app = build_app(state);
    let server = TestServer::new(app).unwrap();

    (server, project.id, pool)
}

/// 清理测试项目数据，级联删除关联的 contracts 和 routes
async fn cleanup(pool: &sqlx::PgPool, project_id: Uuid) {
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project_id)
        .execute(pool)
        .await
        .unwrap();
}

// ── OpenAPI 规范结构测试 ──────────────────────────────────────────────────

#[tokio::test]
async fn docs_openapi_valid_structure() {
    // 验证 OpenAPI 3.0.3 必需的顶层字段全部存在且类型正确
    let server = common::test_server().await;
    let resp = server.get("/api/v1/docs/openapi.json").await;
    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();

    assert_eq!(body["openapi"], "3.0.3", "Must be OpenAPI 3.0.3 version");
    assert!(body["info"].is_object(), "info must be an object");
    assert!(
        body["info"]["title"].is_string(),
        "info.title is required by OpenAPI spec"
    );
    assert!(
        body["info"]["version"].is_string(),
        "info.version is required by OpenAPI spec"
    );
    assert!(body["paths"].is_object(), "paths must be an object");
}

#[tokio::test]
async fn docs_openapi_includes_all_active_routes() {
    // 通过 WSDL 生成路由后，验证 OpenAPI paths 包含所有生成的路由
    let (server, project_id, pool) = server_with_routes().await;

    let resp = server.get("/api/v1/docs/openapi.json").await;
    resp.assert_status(StatusCode::OK);
    let spec: serde_json::Value = resp.json();
    let paths = spec["paths"].as_object().unwrap();

    // calculator.wsdl 会生成 Add 和 GetHistory 两个操作，对应两个路径
    assert!(
        paths.len() >= 2,
        "Should have at least 2 paths from calculator WSDL, got {}",
        paths.len()
    );

    // 验证每个 path 下至少有一个 HTTP method 定义
    for (path, methods) in paths {
        let methods_obj = methods.as_object().unwrap();
        assert!(
            !methods_obj.is_empty(),
            "Path {path} should have at least one HTTP method defined"
        );
    }

    cleanup(&pool, project_id).await;
}

#[tokio::test]
async fn docs_openapi_includes_request_schema() {
    // 验证有 input 参数的操作包含 requestBody 定义
    let (server, project_id, pool) = server_with_routes().await;

    let resp = server.get("/api/v1/docs/openapi.json").await;
    let spec: serde_json::Value = resp.json();
    let paths = spec["paths"].as_object().unwrap();

    // calculator.wsdl 的 Add 操作有 a/b 两个 input 参数，
    // 生成时会写入 request_schema，OpenAPI 中应有 requestBody
    let has_request_body = paths.values().any(|methods| {
        methods.as_object().map_or(false, |m| {
            m.values().any(|op| op.get("requestBody").is_some())
        })
    });
    assert!(
        has_request_body,
        "At least one operation should have requestBody (Add has input params)"
    );

    cleanup(&pool, project_id).await;
}

#[tokio::test]
async fn docs_openapi_includes_error_responses() {
    // 验证每个操作都声明了网关标准错误码 429/502/503/504
    let (server, project_id, pool) = server_with_routes().await;

    let resp = server.get("/api/v1/docs/openapi.json").await;
    let spec: serde_json::Value = resp.json();
    let paths = spec["paths"].as_object().unwrap();

    let expected_error_codes = ["429", "502", "503", "504"];

    for (path, methods) in paths {
        for (method, operation) in methods.as_object().unwrap() {
            let responses = operation["responses"].as_object().unwrap_or_else(|| {
                panic!("{method} {path} must have responses field")
            });
            for code in &expected_error_codes {
                assert!(
                    responses.contains_key(*code),
                    "{method} {path} missing error response {code}"
                );
            }
        }
    }

    cleanup(&pool, project_id).await;
}

#[tokio::test]
async fn docs_openapi_updates_on_new_routes() {
    // 验证新增路由后 OpenAPI 规范动态更新（无缓存失效问题）
    let pool = common::test_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    // 第一次：空路由时获取 OpenAPI
    let router1 = Arc::new(DynamicRouter::new());
    let dispatchers1: Arc<DashMap<Uuid, Arc<BackendDispatcher>>> = Arc::new(DashMap::new());
    let state1 = AppState {
        db: pool.clone(),
        repo: Arc::new(PgMetadataRepo::new(pool.clone())),
        router: router1,
        dispatchers: dispatchers1,
    };
    let server1 = TestServer::new(build_app(state1)).unwrap();
    let resp1 = server1.get("/api/v1/docs/openapi.json").await;
    let spec1: serde_json::Value = resp1.json();
    let paths_count_before = spec1["paths"].as_object().unwrap().len();

    // 生成新路由
    let suffix = Uuid::new_v4();
    let project = repo
        .create_project(
            &format!("docs-update-{suffix}"),
            "test",
            "test",
            SourceType::Wsdl,
        )
        .await
        .unwrap();

    let wsdl = include_str!("../../generator/tests/fixtures/calculator.wsdl").replace(
        "CalculatorService",
        &format!("DocsUpdate{}Service", &suffix.to_string()[..8]),
    );
    api_anything_generator::pipeline::GenerationPipeline::run_wsdl(&repo, project.id, &wsdl, None)
        .await
        .unwrap();

    // 第二次：重新加载路由后获取 OpenAPI
    let router2 = Arc::new(DynamicRouter::new());
    let dispatchers2: Arc<DashMap<Uuid, Arc<BackendDispatcher>>> = Arc::new(DashMap::new());
    RouteLoader::load(&repo, &router2, &dispatchers2)
        .await
        .unwrap();
    let state2 = AppState {
        db: pool.clone(),
        repo: Arc::new(PgMetadataRepo::new(pool.clone())),
        router: router2,
        dispatchers: dispatchers2,
    };
    let server2 = TestServer::new(build_app(state2)).unwrap();
    let resp2 = server2.get("/api/v1/docs/openapi.json").await;
    let spec2: serde_json::Value = resp2.json();
    let paths_count_after = spec2["paths"].as_object().unwrap().len();

    assert!(
        paths_count_after > paths_count_before,
        "OpenAPI should include newly generated routes: before={paths_count_before}, after={paths_count_after}"
    );

    cleanup(&pool, project.id).await;
}

// ── Swagger UI 测试 ──────────────────────────────────────────────────────

#[tokio::test]
async fn docs_swagger_ui_html() {
    let server = common::test_server().await;
    let resp = server.get("/api/v1/docs").await;
    resp.assert_status(StatusCode::OK);
    let body = resp.text();

    // 验证返回的是 HTML 页面
    assert!(
        body.contains("<!DOCTYPE html>") || body.contains("<html"),
        "Swagger UI should return HTML"
    );
    // 验证包含 swagger-ui 组件
    assert!(
        body.contains("swagger-ui"),
        "HTML should reference swagger-ui component"
    );
    // 验证引用了 openapi.json URL
    assert!(
        body.contains("openapi.json"),
        "Swagger UI should reference openapi.json endpoint"
    );
    // 验证包含 SwaggerUIBundle 初始化
    assert!(
        body.contains("SwaggerUIBundle"),
        "HTML should initialize SwaggerUIBundle"
    );
}

// ── Agent Prompt 测试 ────────────────────────────────────────────────────

#[tokio::test]
async fn docs_agent_prompt_markdown() {
    let server = common::test_server().await;
    let resp = server.get("/api/v1/docs/agent-prompt").await;
    resp.assert_status(StatusCode::OK);
    let body = resp.text();
    // 验证包含 Markdown 标题
    assert!(
        body.contains("# API-Anything"),
        "Agent prompt should start with # API-Anything heading"
    );
}

#[tokio::test]
async fn docs_agent_prompt_includes_routes() {
    // 生成路由后，验证 Agent Prompt 中包含路由路径和 HTTP method
    let (server, project_id, pool) = server_with_routes().await;

    let resp = server.get("/api/v1/docs/agent-prompt").await;
    resp.assert_status(StatusCode::OK);
    let body = resp.text();

    // 验证包含 /gw 前缀的路由路径
    assert!(
        body.contains("/gw/"),
        "Agent prompt should list routes with /gw/ prefix"
    );
    // 验证包含 HTTP method（生成的 SOAP 路由都是 POST）
    assert!(
        body.contains("POST"),
        "Agent prompt should include HTTP methods"
    );
    // 验证包含协议类型标注
    assert!(
        body.contains("Protocol"),
        "Agent prompt should mention protocol type"
    );

    cleanup(&pool, project_id).await;
}

#[tokio::test]
async fn docs_agent_prompt_includes_schema_info() {
    // 验证有 request/response schema 的路由在 Agent Prompt 中包含 JSON schema 代码块
    let (server, project_id, pool) = server_with_routes().await;

    let resp = server.get("/api/v1/docs/agent-prompt").await;
    resp.assert_status(StatusCode::OK);
    let body = resp.text();

    // calculator.wsdl 的 Add 操作有 request schema，
    // 生成的 Markdown 应包含 JSON 代码块
    assert!(
        body.contains("```json"),
        "Agent prompt should include JSON code blocks for schemas"
    );
    // 验证包含 Request Body 标注
    assert!(
        body.contains("Request Body"),
        "Agent prompt should label request body sections"
    );

    cleanup(&pool, project_id).await;
}

// ── Health 端点测试 ──────────────────────────────────────────────────────

#[tokio::test]
async fn docs_health_liveness() {
    // /health 只检测进程存活，不依赖数据库
    let server = common::test_server().await;
    let resp = server.get("/health").await;
    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn docs_health_readiness() {
    // /health/ready 验证数据库连通性
    let server = common::test_server().await;
    let resp = server.get("/health/ready").await;
    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();
    assert_eq!(body["status"], "ready");
    assert_eq!(body["db"], "connected");
}
