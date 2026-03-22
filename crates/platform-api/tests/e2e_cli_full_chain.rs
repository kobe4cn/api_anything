// CLI 协议全链路 E2E 测试套件：覆盖从帮助文本解析 → 路由生成 → 网关加载 → 进程执行的完整链路。
// 使用真实系统命令（echo、bash 脚本）验证参数传递、输出解析、
// 安全防护（命令注入）、布尔 flag、数值参数等场景。
// 仅在 Unix 平台运行，因为依赖 bash 和 shebang 语义。
#![allow(unused_imports, dead_code)]
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

// ──────────────────────────────────────────────────────────
// 辅助函数
// ──────────────────────────────────────────────────────────

fn sample_main_help() -> &'static str {
    include_str!("../../generator/tests/fixtures/sample_help.txt")
}

fn sample_sub_help() -> &'static str {
    include_str!("../../generator/tests/fixtures/sample_subcommand_help.txt")
}

/// 获取 mock-report-gen.sh 的绝对路径
#[cfg(unix)]
fn mock_script_path() -> String {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../generator/tests/fixtures/mock-report-gen.sh")
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string()
}

/// 通用辅助：创建项目、运行 CLI pipeline、加载路由、构建测试服务器
#[cfg(unix)]
async fn setup_cli_env(
    program_path: &str,
    main_help: &str,
    sub_helps: &[(&str, &str)],
    project_name: &str,
) -> (TestServer, sqlx::PgPool, Uuid) {
    let pool = common::test_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let project = repo
        .create_project(project_name, "E2E CLI test", "test", SourceType::Cli)
        .await
        .unwrap();

    api_anything_generator::pipeline::GenerationPipeline::run_cli(
        &repo,
        project.id,
        program_path,
        main_help,
        sub_helps,
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
    (server, pool, project.id)
}

/// 测试后清理项目数据
#[cfg(unix)]
async fn cleanup(pool: &sqlx::PgPool, project_id: Uuid) {
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project_id)
        .execute(pool)
        .await
        .unwrap();
}

// ──────────────────────────────────────────────────────────
// 测试用例（仅在 Unix 平台编译和运行）
// ──────────────────────────────────────────────────────────

#[cfg(unix)]
#[tokio::test]
async fn cli_basic_echo_command() {
    // 验证最基础的 CLI 全链路：JSON 请求 → CLI 参数 → 脚本执行 → JSON 响应
    let script = mock_script_path();
    let suffix = Uuid::new_v4().to_string().replace('-', "");
    let short = &suffix[..8];
    let project_name = format!("cli-e2e-basic-{short}");

    let (server, pool, project_id) = setup_cli_env(
        &script,
        sample_main_help(),
        &[("generate", sample_sub_help())],
        &project_name,
    )
    .await;

    // CliMapper 提取脚本 basename 并去除扩展名：mock-report-gen.sh → mock-report-gen
    let resp = server
        .post("/gw/api/v1/mock-report-gen/generate")
        .json(&json!({"type": "daily"}))
        .await;

    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();
    // mock-report-gen.sh 的 generate 子命令输出 JSON，output_format 为 "json" 时直接解析
    assert_eq!(body["report_id"], "R-001");
    assert_eq!(body["status"], "generated");
    assert_eq!(body["type"], "daily");

    cleanup(&pool, project_id).await;
}

#[cfg(unix)]
#[tokio::test]
async fn cli_json_output_parsing() {
    // 验证 output_format 为 json 时，CLI stdout 的 JSON 被直接解析为响应对象，
    // 而非包裹在 {"stdout": "..."} 中
    let script = mock_script_path();
    let suffix = Uuid::new_v4().to_string().replace('-', "");
    let short = &suffix[..8];
    let project_name = format!("cli-e2e-json-{short}");

    let (server, pool, project_id) = setup_cli_env(
        &script,
        sample_main_help(),
        &[("generate", sample_sub_help())],
        &project_name,
    )
    .await;

    let resp = server
        .post("/gw/api/v1/mock-report-gen/generate")
        .json(&json!({"type": "weekly"}))
        .await;

    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();

    // JSON 模式下应直接解析脚本输出，body 中不应有 "stdout" 这个 key
    assert!(
        body.get("stdout").is_none(),
        "JSON 输出模式下不应有 stdout 包装层"
    );
    assert_eq!(body["type"], "weekly");

    cleanup(&pool, project_id).await;
}

#[cfg(unix)]
#[tokio::test]
async fn cli_raw_text_output() {
    // 验证 output_format 为 raw_text 时，响应包含 {"stdout": "..."}
    // 通过直接操作数据库将 output_format 改为 RawText
    let script = mock_script_path();
    let suffix = Uuid::new_v4().to_string().replace('-', "");
    let short = &suffix[..8];
    let project_name = format!("cli-e2e-raw-{short}");

    let pool = common::test_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let project = repo
        .create_project(&project_name, "CLI raw text test", "test", SourceType::Cli)
        .await
        .unwrap();

    api_anything_generator::pipeline::GenerationPipeline::run_cli(
        &repo,
        project.id,
        &script,
        sample_main_help(),
        &[("generate", sample_sub_help())],
    )
    .await
    .unwrap();

    // 将 endpoint_config 中的 output_format 改为 raw text，
    // 验证 OutputParser 的 RawText 分支
    sqlx::query(
        r#"UPDATE backend_bindings SET endpoint_config = jsonb_set(endpoint_config, '{output_format}', '"raw"') WHERE id IN (SELECT backend_binding_id FROM routes WHERE contract_id IN (SELECT id FROM contracts WHERE project_id = $1))"#,
    )
    .bind(project.id)
    .execute(&pool)
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
        .post("/gw/api/v1/mock-report-gen/generate")
        .json(&json!({"type": "daily"}))
        .await;

    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();

    // RawText 模式下应有 stdout 字段
    assert!(
        body["stdout"].is_string(),
        "RawText 模式下应有 stdout 字段，实际 body: {body}"
    );

    cleanup(&pool, project.id).await;
}

#[cfg(unix)]
#[tokio::test]
async fn cli_command_failure_returns_502() {
    // 验证当脚本对未知子命令返回 exit 1 时，网关返回 502 + 错误详情。
    // mock-report-gen.sh 对未知子命令输出 stderr 并 exit 1
    let script = mock_script_path();
    let suffix = Uuid::new_v4().to_string().replace('-', "");
    let short = &suffix[..8];
    let project_name = format!("cli-e2e-fail-{short}");

    let pool = common::test_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let project = repo
        .create_project(&project_name, "CLI failure test", "test", SourceType::Cli)
        .await
        .unwrap();

    api_anything_generator::pipeline::GenerationPipeline::run_cli(
        &repo,
        project.id,
        &script,
        sample_main_help(),
        &[("generate", sample_sub_help())],
    )
    .await
    .unwrap();

    // 将 generate 路由的 subcommand 改为 "unknown_cmd"，触发脚本的 *) 分支
    sqlx::query(
        r#"UPDATE backend_bindings SET endpoint_config = jsonb_set(endpoint_config, '{subcommand}', '"unknown_cmd"') WHERE id IN (SELECT backend_binding_id FROM routes WHERE contract_id IN (SELECT id FROM contracts WHERE project_id = $1) AND path LIKE '%generate%')"#,
    )
    .bind(project.id)
    .execute(&pool)
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
        .post("/gw/api/v1/mock-report-gen/generate")
        .json(&json!({"type": "daily"}))
        .await;

    // CliAdapter 对 exit_code != 0 返回 BackendError{status: 500}，
    // 经 error normalizer 和 IntoResponse 映射后最终为 502 Bad Gateway
    resp.assert_status(StatusCode::BAD_GATEWAY);
    let body: serde_json::Value = resp.json();
    let detail = body["detail"].as_str().unwrap_or("");
    assert!(
        detail.contains("Command failed") || detail.contains("Unknown subcommand"),
        "错误详情应包含命令失败信息，实际: {detail}"
    );

    cleanup(&pool, project.id).await;
}

#[cfg(unix)]
#[tokio::test]
async fn cli_nonexistent_program_returns_502() {
    // 验证当 program 路径指向不存在的程序时，网关返回 502
    let suffix = Uuid::new_v4().to_string().replace('-', "");
    let short = &suffix[..8];
    let project_name = format!("cli-e2e-noexist-{short}");

    let pool = common::test_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let project = repo
        .create_project(&project_name, "CLI nonexistent test", "test", SourceType::Cli)
        .await
        .unwrap();

    // 使用一个合法的帮助文本创建路由，但 program 指向不存在的路径
    api_anything_generator::pipeline::GenerationPipeline::run_cli(
        &repo,
        project.id,
        "/nonexistent/program/xyz",
        sample_main_help(),
        &[("generate", sample_sub_help())],
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

    // CliMapper 对程序路径取 basename 去扩展名：/nonexistent/program/xyz → xyz
    let resp = server
        .post("/gw/api/v1/xyz/generate")
        .json(&json!({"type": "daily"}))
        .await;

    // 程序不存在应返回 502 Bad Gateway
    resp.assert_status(StatusCode::BAD_GATEWAY);
    let body: serde_json::Value = resp.json();
    assert_eq!(body["status"], 502);

    cleanup(&pool, project.id).await;
}

#[cfg(unix)]
#[tokio::test]
async fn cli_command_injection_prevention() {
    // 关键安全测试：验证 shell 元字符不会被 shell 解释执行。
    // CliAdapter 通过 Command::arg() 逐个传参（OS 层面参数隔离），
    // 而非拼接字符串后交给 shell，从而从根本上杜绝命令注入
    let suffix = Uuid::new_v4().to_string().replace('-', "");
    let short = &suffix[..8];
    let project_name = format!("cli-e2e-inject-{short}");

    let pool = common::test_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let project = repo
        .create_project(&project_name, "CLI injection test", "test", SourceType::Cli)
        .await
        .unwrap();

    // 使用 echo 作为 program 来验证参数是否被逐字传递
    api_anything_generator::pipeline::GenerationPipeline::run_cli(
        &repo,
        project.id,
        "echo",
        sample_main_help(),
        &[("generate", sample_sub_help())],
    )
    .await
    .unwrap();

    // 将 output_format 改为 raw 以直接看到 stdout
    sqlx::query(
        r#"UPDATE backend_bindings SET endpoint_config = jsonb_set(endpoint_config, '{output_format}', '"raw"') WHERE id IN (SELECT backend_binding_id FROM routes WHERE contract_id IN (SELECT id FROM contracts WHERE project_id = $1) AND path LIKE '%generate%')"#,
    )
    .bind(project.id)
    .execute(&pool)
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
        .post("/gw/api/v1/echo/generate")
        .json(&json!({"type": "; rm -rf /"}))
        .await;

    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();
    let stdout = body["stdout"].as_str().unwrap_or("");
    // echo 应该原样输出注入 payload，而非执行它
    assert!(
        stdout.contains("; rm -rf /"),
        "注入 payload 应被作为字面参数输出，实际 stdout: {stdout}"
    );

    cleanup(&pool, project.id).await;
}

#[cfg(unix)]
#[tokio::test]
async fn cli_command_injection_variants() {
    // 验证多种常见命令注入 payload 均被作为字面参数传递
    let suffix = Uuid::new_v4().to_string().replace('-', "");
    let short = &suffix[..8];
    let project_name = format!("cli-e2e-inject-v-{short}");

    let pool = common::test_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let project = repo
        .create_project(&project_name, "CLI injection variants", "test", SourceType::Cli)
        .await
        .unwrap();

    api_anything_generator::pipeline::GenerationPipeline::run_cli(
        &repo,
        project.id,
        "echo",
        sample_main_help(),
        &[("generate", sample_sub_help())],
    )
    .await
    .unwrap();

    // 使用 raw 输出格式直接查看 stdout
    sqlx::query(
        r#"UPDATE backend_bindings SET endpoint_config = jsonb_set(endpoint_config, '{output_format}', '"raw"') WHERE id IN (SELECT backend_binding_id FROM routes WHERE contract_id IN (SELECT id FROM contracts WHERE project_id = $1) AND path LIKE '%generate%')"#,
    )
    .bind(project.id)
    .execute(&pool)
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

    // 逐一测试各种注入 payload
    let payloads = vec![
        "$(whoami)",
        "`id`",
        "| cat /etc/passwd",
        "&& echo hacked",
    ];

    for payload in payloads {
        let resp = server
            .post("/gw/api/v1/echo/generate")
            .json(&json!({"type": payload}))
            .await;

        resp.assert_status(StatusCode::OK);
        let body: serde_json::Value = resp.json();
        let stdout = body["stdout"].as_str().unwrap_or("");
        assert!(
            stdout.contains(payload),
            "注入 payload '{payload}' 应被原样输出，实际 stdout: {stdout}"
        );
    }

    cleanup(&pool, project.id).await;
}

#[cfg(unix)]
#[tokio::test]
async fn cli_boolean_flag_handling() {
    // 验证 JSON body 中的布尔值被正确映射为 CLI flag：
    // true → --flag（仅 key，无 value）
    // false → 跳过（不传该 flag）
    let suffix = Uuid::new_v4().to_string().replace('-', "");
    let short = &suffix[..8];
    let project_name = format!("cli-e2e-bool-{short}");

    let pool = common::test_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let project = repo
        .create_project(&project_name, "CLI bool flag test", "test", SourceType::Cli)
        .await
        .unwrap();

    // 使用 echo 来可视化传递的参数
    api_anything_generator::pipeline::GenerationPipeline::run_cli(
        &repo,
        project.id,
        "echo",
        sample_main_help(),
        &[("generate", sample_sub_help())],
    )
    .await
    .unwrap();

    sqlx::query(
        r#"UPDATE backend_bindings SET endpoint_config = jsonb_set(endpoint_config, '{output_format}', '"raw"') WHERE id IN (SELECT backend_binding_id FROM routes WHERE contract_id IN (SELECT id FROM contracts WHERE project_id = $1) AND path LIKE '%generate%')"#,
    )
    .bind(project.id)
    .execute(&pool)
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
        .post("/gw/api/v1/echo/generate")
        .json(&json!({"verbose": true, "quiet": false}))
        .await;

    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();
    let stdout = body["stdout"].as_str().unwrap_or("");

    // true → --verbose 应出现在输出中
    assert!(
        stdout.contains("--verbose"),
        "布尔 true 应生成 --verbose flag，实际 stdout: {stdout}"
    );
    // false → --quiet 不应出现
    assert!(
        !stdout.contains("--quiet"),
        "布尔 false 应被跳过，不应出现 --quiet，实际 stdout: {stdout}"
    );

    cleanup(&pool, project.id).await;
}

#[cfg(unix)]
#[tokio::test]
async fn cli_numeric_parameter() {
    // 验证数值参数被正确转为 --key value 形式
    let suffix = Uuid::new_v4().to_string().replace('-', "");
    let short = &suffix[..8];
    let project_name = format!("cli-e2e-num-{short}");

    let pool = common::test_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let project = repo
        .create_project(&project_name, "CLI numeric test", "test", SourceType::Cli)
        .await
        .unwrap();

    api_anything_generator::pipeline::GenerationPipeline::run_cli(
        &repo,
        project.id,
        "echo",
        sample_main_help(),
        &[("generate", sample_sub_help())],
    )
    .await
    .unwrap();

    sqlx::query(
        r#"UPDATE backend_bindings SET endpoint_config = jsonb_set(endpoint_config, '{output_format}', '"raw"') WHERE id IN (SELECT backend_binding_id FROM routes WHERE contract_id IN (SELECT id FROM contracts WHERE project_id = $1) AND path LIKE '%generate%')"#,
    )
    .bind(project.id)
    .execute(&pool)
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
        .post("/gw/api/v1/echo/generate")
        .json(&json!({"count": 5, "name": "test"}))
        .await;

    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();
    let stdout = body["stdout"].as_str().unwrap_or("");

    // 数值参数应转为 --count 5
    assert!(
        stdout.contains("--count") && stdout.contains("5"),
        "应包含 --count 5，实际 stdout: {stdout}"
    );
    // 字符串参数应转为 --name test
    assert!(
        stdout.contains("--name") && stdout.contains("test"),
        "应包含 --name test，实际 stdout: {stdout}"
    );

    cleanup(&pool, project.id).await;
}

#[cfg(unix)]
#[tokio::test]
async fn cli_http_method_inference() {
    // 验证 CliMapper 根据子命令名称推断 HTTP 方法：
    // generate → POST（创建类动词），list → GET（查询类动词）
    let script = mock_script_path();
    let suffix = Uuid::new_v4().to_string().replace('-', "");
    let short = &suffix[..8];
    let project_name = format!("cli-e2e-method-{short}");

    let pool = common::test_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let project = repo
        .create_project(&project_name, "CLI method inference test", "test", SourceType::Cli)
        .await
        .unwrap();

    api_anything_generator::pipeline::GenerationPipeline::run_cli(
        &repo,
        project.id,
        &script,
        sample_main_help(),
        &[("generate", sample_sub_help())],
    )
    .await
    .unwrap();

    // 查询数据库中路由的 HTTP 方法
    let routes: Vec<(String, String)> = sqlx::query_as(
        "SELECT path, method::text FROM routes WHERE contract_id IN (SELECT id FROM contracts WHERE project_id = $1) ORDER BY path",
    )
    .bind(project.id)
    .fetch_all(&pool)
    .await
    .unwrap();

    // 验证各子命令的 HTTP 方法推断
    for (path, http_method) in &routes {
        if path.contains("generate") {
            assert_eq!(http_method.to_uppercase(), "POST", "generate 应推断为 POST");
        } else if path.contains("list") {
            assert_eq!(http_method.to_uppercase(), "GET", "list 应推断为 GET");
        } else if path.contains("export") {
            // export 不在查询/创建/删除/更新类动词中，兜底为 POST
            assert_eq!(http_method.to_uppercase(), "POST", "export 应兜底为 POST");
        }
    }

    // 额外验证：通过 TestServer 发送 GET 请求访问 list 路由
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

    // list 子命令应通过 GET 访问
    let resp = server
        .get("/gw/api/v1/mock-report-gen/list")
        .await;
    resp.assert_status(StatusCode::OK);

    // generate 子命令应通过 POST 访问
    let resp = server
        .post("/gw/api/v1/mock-report-gen/generate")
        .json(&json!({"type": "daily"}))
        .await;
    resp.assert_status(StatusCode::OK);

    cleanup(&pool, project.id).await;
}
