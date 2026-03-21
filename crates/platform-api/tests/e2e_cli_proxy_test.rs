/// E2E 集成测试：验证完整的 CLI 帮助解析 → 生成 → 加载 → 网关代理链路。
/// 使用 shell 脚本模拟真实 CLI 工具，无需外部服务；
/// 仅在 Unix 平台运行，因为依赖 bash 和 shebang 语义
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
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

mod common;

#[cfg(unix)]
#[tokio::test]
async fn e2e_cli_generate_load_and_proxy() {
    let pool = common::test_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let suffix = Uuid::new_v4();

    // ── Step 1: 创建项目 ────────────────────────────────────────────────────
    let project = repo
        .create_project(
            &format!("cli-e2e-{suffix}"),
            "CLI E2E proxy test",
            "test",
            SourceType::Cli,
        )
        .await
        .unwrap();

    // ── Step 2: 定位 mock 脚本的绝对路径 ────────────────────────────────────
    // canonicalize 解析 ../.. 等相对符，确保路径在任意工作目录下均有效；
    // CARGO_MANIFEST_DIR 在编译期展开为当前 crate 的目录
    let script_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../generator/tests/fixtures/mock-report-gen.sh")
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string();

    // ── Step 3: 运行 CLI 生成流水线 ─────────────────────────────────────────
    let main_help = include_str!("../../generator/tests/fixtures/sample_help.txt");
    let sub_help = include_str!("../../generator/tests/fixtures/sample_subcommand_help.txt");

    let result = api_anything_generator::pipeline::GenerationPipeline::run_cli(
        &repo,
        project.id,
        &script_path,
        main_help,
        &[("generate", sub_help)],
    )
    .await
    .unwrap();

    // sample_help.txt 定义了 generate / list / export 三个子命令
    assert!(
        result.routes_count >= 1,
        "Expected at least 1 route, got {}",
        result.routes_count
    );

    // ── Step 4: 将数据库路由加载到网关 ─────────────────────────────────────
    let router = Arc::new(DynamicRouter::new());
    let dispatchers: Arc<DashMap<Uuid, Arc<BackendDispatcher>>> = Arc::new(DashMap::new());
    RouteLoader::load(&repo, &router, &dispatchers).await.unwrap();

    // ── Step 5: 构建 Axum 测试服务器 ────────────────────────────────────────
    let state = AppState {
        db: pool.clone(),
        repo: Arc::new(PgMetadataRepo::new(pool.clone())),
        router,
        dispatchers,
    };
    let app = build_app(state);
    let server = TestServer::new(app).unwrap();

    // ── Step 6: 向 generate 路由发送 POST 请求 ──────────────────────────────
    // CliMapper 提取脚本 basename 并去除扩展名作为 slug：
    // mock-report-gen.sh → mock-report-gen，生成路径 /api/v1/mock-report-gen/generate；
    // 网关在其前面加 /gw 前缀
    let resp = server
        .post("/gw/api/v1/mock-report-gen/generate")
        .json(&json!({"type": "daily"}))
        .await;

    // ── Step 7: 验证响应状态码 ─────────────────────────────────────────────
    // mock 脚本输出合法 JSON 且退出码为 0；
    // output_format 为 "json"（pipeline 写入）时响应体为解析后的 JSON 对象，
    // 为 RawText 时响应体为 {"stdout": "..."}，两种情况下 HTTP 状态均应为 200
    resp.assert_status(StatusCode::OK);

    // ── Step 8: 清理本次测试写入的数据 ─────────────────────────────────────
    // 级联删除 contracts → routes；backend_bindings 变为孤立记录但不影响测试隔离
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project.id)
        .execute(&pool)
        .await
        .unwrap();
}
