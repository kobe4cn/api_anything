/// 文档端点集成测试：验证 OpenAPI JSON、Swagger UI、Agent Prompt 三个端点的基本正确性；
/// openapi_includes_gateway_routes 需要真实数据库，通过生成流水线写入路由后验证规范内容
use api_anything_metadata::MetadataRepo;
use axum::http::StatusCode;

mod common;

#[tokio::test]
async fn openapi_json_returns_valid_spec() {
    let server = common::test_server().await;
    let resp = server.get("/api/v1/docs/openapi.json").await;
    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();
    assert_eq!(body["openapi"], "3.0.3");
    assert!(body["info"]["title"].is_string());
    assert!(body["paths"].is_object());
}

#[tokio::test]
async fn swagger_ui_returns_html() {
    let server = common::test_server().await;
    let resp = server.get("/api/v1/docs").await;
    resp.assert_status(StatusCode::OK);
    let body = resp.text();
    assert!(body.contains("swagger-ui"));
    assert!(body.contains("openapi.json"));
}

#[tokio::test]
async fn openapi_includes_gateway_routes() {
    // 先执行生成流水线写入路由，再验证 OpenAPI 规范中包含对应路径；
    // 通过 UUID 后缀避免并发测试间的项目名冲突
    let pool = common::test_pool().await;
    let repo = api_anything_metadata::PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let suffix = uuid::Uuid::new_v4();
    let project = repo
        .create_project(
            &format!("docs-test-{suffix}"),
            "test",
            "team",
            api_anything_common::models::SourceType::Wsdl,
        )
        .await
        .unwrap();

    // 替换 fixture 中的服务名称，防止与其他测试产生操作名冲突
    let wsdl = include_str!("../../generator/tests/fixtures/calculator.wsdl")
        .replace(
            "CalculatorService",
            &format!("DocTest{}Service", &suffix.to_string()[..8]),
        );
    api_anything_generator::pipeline::GenerationPipeline::run_wsdl(&repo, project.id, &wsdl)
        .await
        .unwrap();

    // 加载路由到内存路由表后构建 app，确保 openapi_json handler 能查到刚生成的路由
    let router = std::sync::Arc::new(api_anything_gateway::router::DynamicRouter::new());
    let dispatchers = std::sync::Arc::new(dashmap::DashMap::new());
    api_anything_gateway::loader::RouteLoader::load(&repo, &router, &dispatchers)
        .await
        .unwrap();

    let state = api_anything_platform_api::state::AppState {
        db: pool.clone(),
        repo: std::sync::Arc::new(api_anything_metadata::PgMetadataRepo::new(pool.clone())),
        router,
        dispatchers,
    };
    let app = api_anything_platform_api::build_app(state);
    let server = axum_test::TestServer::new(app).unwrap();

    let resp = server.get("/api/v1/docs/openapi.json").await;
    let spec: serde_json::Value = resp.json();
    let paths = spec["paths"].as_object().unwrap();
    assert!(!paths.is_empty(), "Should have paths from generated routes");

    // 清理测试数据，防止测试数据库积累孤立项目
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project.id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn agent_prompt_returns_markdown() {
    let server = common::test_server().await;
    let resp = server.get("/api/v1/docs/agent-prompt").await;
    resp.assert_status(StatusCode::OK);
    let body = resp.text();
    assert!(body.contains("# API-Anything"));
}

#[tokio::test]
async fn agent_prompt_includes_routes() {
    // 与 openapi_includes_gateway_routes 使用相同的数据构造方式，
    // 但验证 Markdown 输出中包含路由路径，确保 Agent Prompt 格式正确
    let pool = common::test_pool().await;
    let repo = api_anything_metadata::PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let suffix = uuid::Uuid::new_v4();
    let project = repo
        .create_project(
            &format!("prompt-test-{suffix}"),
            "test",
            "team",
            api_anything_common::models::SourceType::Wsdl,
        )
        .await
        .unwrap();

    let wsdl = include_str!("../../generator/tests/fixtures/calculator.wsdl").replace(
        "CalculatorService",
        &format!("PromptTest{}Service", &suffix.to_string()[..8]),
    );
    api_anything_generator::pipeline::GenerationPipeline::run_wsdl(&repo, project.id, &wsdl)
        .await
        .unwrap();

    let router = std::sync::Arc::new(api_anything_gateway::router::DynamicRouter::new());
    let dispatchers = std::sync::Arc::new(dashmap::DashMap::new());
    api_anything_gateway::loader::RouteLoader::load(&repo, &router, &dispatchers)
        .await
        .unwrap();

    let state = api_anything_platform_api::state::AppState {
        db: pool.clone(),
        repo: std::sync::Arc::new(api_anything_metadata::PgMetadataRepo::new(pool.clone())),
        router,
        dispatchers,
    };
    let app = api_anything_platform_api::build_app(state);
    let server = axum_test::TestServer::new(app).unwrap();

    let resp = server.get("/api/v1/docs/agent-prompt").await;
    resp.assert_status(StatusCode::OK);
    let body = resp.text();
    // 验证生成的路由路径出现在 Markdown 输出中（/gw 前缀 + 路由路径）
    assert!(body.contains("/gw/"), "Prompt should contain /gw/ prefixed route paths");

    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project.id)
        .execute(&pool)
        .await
        .unwrap();
}
