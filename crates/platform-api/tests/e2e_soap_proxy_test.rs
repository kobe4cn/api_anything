/// E2E 集成测试：验证完整的 WSDL → 生成 → 加载 → 网关代理链路。
/// 使用 wiremock 模拟真实 SOAP 后端，无需外部服务，也不依赖网络；
/// 这个测试覆盖了 Phase 1 的全部核心流程，是最高价值的回归保护
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
use wiremock::matchers::{header, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

mod common;

/// 构造 Add 操作的 SOAP 响应 XML；result 是字符串类型，
/// 与 SoapXmlParser 的行为一致（所有叶节点文本均解析为 JSON string）
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

/// 读取 calculator.wsdl fixture，通过相对路径引用 generator crate 的测试资产；
/// include_str! 在编译期展开，确保路径错误在构建阶段暴露而非运行时
fn calculator_wsdl() -> String {
    include_str!("../../generator/tests/fixtures/calculator.wsdl").to_string()
}

#[tokio::test]
async fn e2e_wsdl_generate_load_and_proxy() {
    // ── Step 1: 启动 wiremock 模拟 SOAP 后端 ──────────────────────────────────
    // MockServer 随机绑定端口，避免与其他测试或服务冲突
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("Content-Type", "text/xml; charset=utf-8"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(soap_add_response(8)),
        )
        // 至少被调用一次，测试结束时 MockServer drop 会自动验证此断言
        .expect(1..)
        .mount(&mock_server)
        .await;

    // ── Step 2: 修改 WSDL ─────────────────────────────────────────────────────
    // (a) 用唯一 slug 替换服务名：使生成的路径包含 UUID，
    //     避免与其他测试遗留的 /api/v1/calculator/add 路由冲突；
    //     RouteLoader 加载所有活跃路由时，相同路径最后一次插入胜出，
    //     若路径唯一则无此竞争问题
    // (b) 仅替换 soap:address location，不影响 targetNamespace 命名空间声明；
    //     保持原始 namespace 有助于验证 SoapXmlBuilder 的命名空间写入逻辑
    let suffix = Uuid::new_v4();
    // 将 UUID 的连字符去掉，仅保留 16 位十六进制前缀，适合嵌入 XML name 属性
    let unique_slug = format!("e2esvc{}", &suffix.to_string().replace('-', "")[..16]);

    let wsdl = calculator_wsdl()
        // 替换 WSDL 定义根元素的 name 属性（service name → 用于路径生成）
        .replace(
            r#"name="CalculatorService""#,
            &format!(r#"name="{unique_slug}Service""#),
        )
        // 替换 soap:address endpoint，指向本次测试专用的 mock server
        .replace(
            r#"location="http://example.com/calculator""#,
            &format!(r#"location="{}""#, mock_server.uri()),
        );

    // ── Step 3: 初始化数据库并运行生成流水线 ──────────────────────────────────
    let pool = common::test_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    // run_migrations 是幂等的，重复调用不会报错
    repo.run_migrations().await.unwrap();

    // 项目名也包含 UUID，防止 name UNIQUE 约束冲突
    let project = repo
        .create_project(
            &format!("e2e-soap-{suffix}"),
            "E2E SOAP proxy test",
            "test",
            SourceType::Wsdl,
        )
        .await
        .unwrap();

    let result = api_anything_generator::pipeline::GenerationPipeline::run_wsdl(
        &repo,
        project.id,
        &wsdl,
        None,
    )
    .await
    .unwrap();
    // calculator.wsdl 定义了 Add 和 GetHistory 两个操作
    assert_eq!(result.routes_count, 2, "Should generate 2 routes from calculator.wsdl");

    // WsdlMapper 将服务名转为 kebab-case 并去掉 "-service" 后缀；
    // 例如 "E2esvc0123456789abcdefService" → "e2esvc0123456789abcdef"
    // 操作 "Add" → "add"，生成路径 /api/v1/{unique_slug}/add
    let kebab_slug = to_kebab_strip_service(&format!("{unique_slug}Service"));
    let expected_path = format!("/gw/api/v1/{kebab_slug}/add");

    // ── Step 4: 将数据库路由加载到网关 ────────────────────────────────────────
    let router = Arc::new(DynamicRouter::new());
    let dispatchers: Arc<DashMap<Uuid, Arc<BackendDispatcher>>> = Arc::new(DashMap::new());
    let loaded = RouteLoader::load(&repo, &router, &dispatchers).await.unwrap();
    // 数据库可能存有其他测试留下的路由，只需确认至少加载了本测试创建的 2 条
    assert!(
        loaded >= 2,
        "Should load at least 2 routes, got {loaded}"
    );

    // ── Step 5: 构建 Axum 应用并创建测试服务器 ─────────────────────────────────
    let state = AppState {
        db: pool.clone(),
        repo: Arc::new(repo),
        router,
        dispatchers,
    };
    let app = build_app(state);
    let server = TestServer::new(app).unwrap();

    // ── Step 6: 向网关发送 JSON 请求 ──────────────────────────────────────────
    // 路径为本次测试专用的唯一路径，不会命中任何其他测试的路由
    let resp = server
        .post(&expected_path)
        .json(&json!({"a": 5, "b": 3}))
        .await;

    // ── Step 7: 验证响应 ───────────────────────────────────────────────────────
    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();
    // SoapXmlParser 将 XML 文本节点解析为 JSON string，不是 number
    assert_eq!(
        body["result"], "8",
        "Expected result=8 from mock SOAP server, got: {body}"
    );

    // ── Step 8: 清理本次测试写入的数据 ───────────────────────────────────────
    // 级联删除 contracts → routes；backend_bindings 变为孤立记录但不影响测试隔离
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project.id)
        .execute(&pool)
        .await
        .unwrap();

    // MockServer 在此 drop，wiremock 自动验证 expect(1..) 断言：
    // 如果没有收到任何 SOAP 请求，测试将在此处 panic
}

/// 复现 WsdlMapper::to_kebab_case 的逻辑并去掉 "-service" 后缀；
/// 用于在测试中预测生成路由的路径，无需访问内部实现
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
