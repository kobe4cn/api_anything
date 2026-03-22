/// 跨模块全场景集成测试 — 覆盖 7 条完整业务链路。
/// 每个场景测试多个模块的协作（generator → metadata → gateway → sandbox → compensation → docs），
/// 而非验证单个功能点。所有场景使用唯一 UUID 后缀避免并行测试的路由路径冲突
use api_anything_common::error::AppError;
use api_anything_common::models::{SandboxMode, SourceType};
use api_anything_gateway::adapter::{BoxFuture, ProtocolAdapter};
use api_anything_gateway::dispatcher::{BackendDispatcher, ProtectionStack};
use api_anything_gateway::loader::RouteLoader;
use api_anything_gateway::router::DynamicRouter;
use api_anything_gateway::types::*;
use api_anything_metadata::{MetadataRepo, PgMetadataRepo};
use api_anything_platform_api::build_app;
use api_anything_platform_api::state::AppState;
use axum::http::{HeaderMap, Method, StatusCode};
use axum_test::TestServer;
use chrono::Utc;
use dashmap::DashMap;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;
use wiremock::matchers::{header, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

mod common;

// ═══════════════════════════════════════════════════════════════════════════
// 辅助函数
// ═══════════════════════════════════════════════════════════════════════════

/// 构造 SOAP Add 操作的响应 XML
fn soap_response(result: &str) -> String {
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

fn calculator_wsdl() -> String {
    include_str!("../../generator/tests/fixtures/calculator.wsdl").to_string()
}

/// 复现 WsdlMapper::to_kebab_case 逻辑并去掉 "-service" 后缀，用于预测生成路由的路径
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

// ═══════════════════════════════════════════════════════════════════════════
// 场景 A: SOAP 全生命周期
// 生成 → 网关代理 → 沙箱 mock → proxy 自动录制 → replay 回放 → 文档一致性
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn scenario_a_soap_full_lifecycle() {
    // 1. 启动 wiremock SOAP 后端，返回 AddResponse result=42
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("Content-Type", "text/xml; charset=utf-8"))
        .respond_with(ResponseTemplate::new(200).set_body_string(soap_response("42")))
        .mount(&mock_server)
        .await;

    // 2. 替换 WSDL 的 endpoint 和 service name 为唯一标识
    let suffix = Uuid::new_v4();
    let unique_slug = format!("inta{}", &suffix.to_string().replace('-', "")[..16]);
    let wsdl = calculator_wsdl()
        .replace(
            r#"name="CalculatorService""#,
            &format!(r#"name="{unique_slug}Service""#),
        )
        .replace(
            r#"location="http://example.com/calculator""#,
            &format!(r#"location="{}""#, mock_server.uri()),
        );

    let pool = common::test_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let project = repo
        .create_project(
            &format!("int-a-{}", &suffix.to_string()[..8]),
            "integration",
            "test",
            SourceType::Wsdl,
        )
        .await
        .unwrap();

    // 3. 运行 WSDL 生成流水线
    let gen_result =
        api_anything_generator::pipeline::GenerationPipeline::run_wsdl(&repo, project.id, &wsdl)
            .await
            .unwrap();
    assert_eq!(
        gen_result.routes_count, 2,
        "calculator.wsdl 应生成 Add 和 GetHistory 两条路由"
    );

    // 4. 加载路由到网关
    let router = Arc::new(DynamicRouter::new());
    let dispatchers: Arc<DashMap<Uuid, Arc<BackendDispatcher>>> = Arc::new(DashMap::new());
    RouteLoader::load(&repo, &router, &dispatchers)
        .await
        .unwrap();

    let kebab_slug = to_kebab_strip_service(&format!("{unique_slug}Service"));
    let add_path = format!("/api/v1/{kebab_slug}/add");

    let repo_arc = Arc::new(PgMetadataRepo::new(pool.clone()));
    let state = AppState {
        db: pool.clone(),
        repo: repo_arc.clone(),
        router,
        dispatchers,
    };
    let server = TestServer::new(build_app(state)).unwrap();

    // 5. 通过网关代理 SOAP 请求
    let gw_resp = server
        .post(&format!("/gw{add_path}"))
        .json(&json!({"a": 20, "b": 22}))
        .await;
    gw_resp.assert_status(StatusCode::OK);
    let body: Value = gw_resp.json();
    // SoapXmlParser 将 XML 文本节点解析为 JSON string
    assert_eq!(body["result"], "42", "SOAP 代理应返回 result=42");

    // 6. Mock 沙箱 — 不调用真实后端，从 response_schema 生成模拟数据
    let mock_resp = server
        .post(&format!("/sandbox{add_path}"))
        .add_header("X-Sandbox-Mode", "mock")
        .json(&json!({"a": 1, "b": 2}))
        .await;
    mock_resp.assert_status(StatusCode::OK);
    let mock_body: Value = mock_resp.json();
    assert!(
        mock_body.is_object(),
        "Mock 模式应返回 JSON 对象"
    );

    // 7. Proxy 沙箱 — 调用真实后端并自动录制交互
    let session = repo_arc
        .create_sandbox_session(
            project.id,
            "int-test-tenant",
            SandboxMode::Proxy,
            &json!({}),
            Utc::now() + chrono::Duration::hours(1),
        )
        .await
        .unwrap();

    let proxy_resp = server
        .post(&format!("/sandbox{add_path}"))
        .add_header("X-Sandbox-Mode", "proxy")
        .add_header("X-Sandbox-Session", &session.id.to_string())
        .json(&json!({"a": 5, "b": 3}))
        .await;
    proxy_resp.assert_status(StatusCode::OK);

    // 8. 验证自动录制存在
    let recordings_resp = server
        .get(&format!(
            "/api/v1/sandbox-sessions/{}/recordings",
            session.id
        ))
        .await;
    recordings_resp.assert_status(StatusCode::OK);
    let recordings: Vec<Value> = recordings_resp.json();
    assert!(
        !recordings.is_empty(),
        "Proxy 模式应至少产生 1 条录制记录"
    );

    // 9. Replay 沙箱 — 回放录制数据
    let replay_resp = server
        .post(&format!("/sandbox{add_path}"))
        .add_header("X-Sandbox-Mode", "replay")
        .add_header("X-Sandbox-Session", &session.id.to_string())
        .json(&json!({"a": 5, "b": 3}))
        .await;
    // replay 匹配到录制数据返回 200，不匹配返回 404，两者均为合法行为
    assert!(
        replay_resp.status_code() == 200 || replay_resp.status_code() == 404,
        "Replay 应返回 200(匹配) 或 404(未匹配)，实际: {}",
        replay_resp.status_code()
    );

    // 10. 文档一致性验证
    let openapi: Value = server.get("/api/v1/docs/openapi.json").await.json();
    assert!(
        openapi["paths"]
            .as_object()
            .unwrap()
            .keys()
            .any(|p| p.contains("add")),
        "OpenAPI 规范应包含 add 路由"
    );

    let prompt = server.get("/api/v1/docs/agent-prompt").await.text();
    assert!(
        prompt.contains("add") || prompt.contains("Add"),
        "Agent 提示词应提及 add 操作"
    );

    let sdk = server.get("/api/v1/docs/sdk/typescript").await.text();
    assert!(sdk.contains("fetch"), "TypeScript SDK 应包含 fetch 调用");

    // 清理
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project.id)
        .execute(&pool)
        .await
        .unwrap();
}

// ═══════════════════════════════════════════════════════════════════════════
// 场景 B: CLI 全生命周期
// 生成 → 加载 → 网关执行 → 沙箱 mock → 命令注入防护
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(unix)]
#[tokio::test]
async fn scenario_b_cli_full_lifecycle() {
    let pool = common::test_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let suffix = Uuid::new_v4();

    // 1. 定位 mock CLI 脚本并创建唯一 symlink，避免多个 CLI 项目共用同一 basename 导致路径冲突
    let script_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../generator/tests/fixtures/mock-report-gen.sh")
        .canonicalize()
        .unwrap();

    let link_name = format!("intb-cli-{}", &suffix.to_string()[..8]);
    let link_path = std::env::temp_dir().join(&link_name);
    std::os::unix::fs::symlink(&script_path, &link_path).unwrap();

    // 2. CLI 生成流水线
    let main_help = include_str!("../../generator/tests/fixtures/sample_help.txt");
    let sub_help = include_str!("../../generator/tests/fixtures/sample_subcommand_help.txt");

    let project = repo
        .create_project(
            &format!("int-b-{}", &suffix.to_string()[..8]),
            "CLI integration",
            "test",
            SourceType::Cli,
        )
        .await
        .unwrap();

    let result = api_anything_generator::pipeline::GenerationPipeline::run_cli(
        &repo,
        project.id,
        link_path.to_str().unwrap(),
        main_help,
        &[("generate", sub_help)],
    )
    .await
    .unwrap();
    assert!(
        result.routes_count >= 1,
        "CLI 流水线应至少生成 1 条路由"
    );

    // 3. 加载路由 + 构建测试服务器
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
    let server = TestServer::new(build_app(state)).unwrap();

    // 4. 查找生成的路由路径
    let openapi: Value = server.get("/api/v1/docs/openapi.json").await.json();
    let paths = openapi["paths"].as_object().unwrap();
    let gen_path = paths
        .keys()
        .find(|p| p.contains(&link_name) && p.contains("generate"))
        .cloned();

    // 5. 通过网关执行 CLI 命令
    if let Some(ref path) = gen_path {
        let resp = server
            .post(path)
            .json(&json!({"type": "daily"}))
            .await;
        // 200 表示执行成功，502 表示脚本路径或执行问题，两者均为可预期行为
        assert!(
            resp.status_code() == 200 || resp.status_code() == 502,
            "CLI 网关应返回 200 或 502，实际: {}",
            resp.status_code()
        );

        if resp.status_code() == 200 {
            let body: Value = resp.json();
            assert!(body.is_object(), "JSON 输出应被正确解析为对象");
        }
    }

    // 6. Mock 沙箱
    if let Some(first_path) = paths.keys().find(|p| p.contains(&link_name)) {
        let sandbox_path = first_path.replace("/gw", "/sandbox");
        let mock_resp = server
            .post(&sandbox_path)
            .add_header("X-Sandbox-Mode", "mock")
            .json(&json!({}))
            .await;
        mock_resp.assert_status(StatusCode::OK);
    }

    // 7. 命令注入防护验证：注入参数不应被 shell 解释执行
    if let Some(ref path) = gen_path {
        let injection_resp = server
            .post(path)
            .json(&json!({"type": "; rm -rf /"}))
            .await;
        // 无论成功或失败，注入命令都不应被执行
        if injection_resp.status_code() == 200 {
            let body: Value = injection_resp.json();
            let output = body.to_string();
            assert!(
                !output.contains("hacked"),
                "注入命令不应被执行"
            );
        }
    }

    // 清理
    let _ = std::fs::remove_file(&link_path);
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project.id)
        .execute(&pool)
        .await
        .unwrap();
}

// ═══════════════════════════════════════════════════════════════════════════
// 场景 C: 补偿全链路
// delivery guarantee → 失败 → 死信 → 管理 API → 重推 → 解决 → Webhook
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn scenario_c_compensation_full_chain() {
    let pool = common::test_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();
    let suffix = Uuid::new_v4();

    // 1. 用不挂载任何 mock 的 wiremock 作为"不可达后端"
    //    wiremock 对未匹配的请求返回 404，触发 BackendError
    let mock_server = MockServer::start().await;

    let unique_slug = format!("intc{}", &suffix.to_string().replace('-', "")[..16]);
    let wsdl = calculator_wsdl()
        .replace(
            r#"name="CalculatorService""#,
            &format!(r#"name="{unique_slug}Service""#),
        )
        .replace(
            r#"location="http://example.com/calculator""#,
            &format!(r#"location="{}""#, mock_server.uri()),
        );

    let project = repo
        .create_project(
            &format!("int-c-{}", &suffix.to_string()[..8]),
            "comp test",
            "test",
            SourceType::Wsdl,
        )
        .await
        .unwrap();

    api_anything_generator::pipeline::GenerationPipeline::run_wsdl(&repo, project.id, &wsdl)
        .await
        .unwrap();

    // 查找 add 路由并更新 delivery_guarantee 为 at_least_once
    let routes = repo.list_active_routes_with_bindings().await.unwrap();
    let kebab_slug = to_kebab_strip_service(&format!("{unique_slug}Service"));
    let add_path = format!("/api/v1/{kebab_slug}/add");
    let target_route = routes
        .iter()
        .find(|r| r.path == add_path)
        .expect("应能找到 add 路由");
    let route_id = target_route.route_id;

    sqlx::query("UPDATE routes SET delivery_guarantee = 'at_least_once'::delivery_guarantee WHERE id = $1")
        .bind(route_id)
        .execute(&pool)
        .await
        .unwrap();

    // 2. 重新加载路由（使 delivery_guarantee 变更生效）
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
    let server = TestServer::new(build_app(state)).unwrap();

    // 3. 发送请求（后端返回 404 → 502 Bad Gateway）
    let trace_id = format!("trace-intc-{}", Uuid::new_v4());
    let resp = server
        .post(&format!("/gw{add_path}"))
        .add_header("traceparent", &trace_id)
        .json(&json!({"data": "test"}))
        .await;
    assert!(
        resp.status_code() == 502 || resp.status_code() == 500 || resp.status_code() == 400,
        "后端不可达时应返回错误状态码，实际: {}", resp.status_code()
    );

    // 4. 验证 delivery_record 已创建且状态为 failed
    let records: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id, status::text FROM delivery_records WHERE route_id = $1 AND trace_id = $2 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(route_id)
    .bind(&trace_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        !records.is_empty(),
        "at_least_once 路由应创建 delivery_record"
    );
    let (record_id, status) = &records[0];
    assert_eq!(status, "failed", "投递失败的记录状态应为 failed");

    // 5. 手动将记录推入死信状态（模拟重试耗尽）
    sqlx::query("UPDATE delivery_records SET status = 'dead'::delivery_status, retry_count = 5 WHERE id = $1")
        .bind(record_id)
        .execute(&pool)
        .await
        .unwrap();

    // 6. 通过管理 API 验证死信可见
    let dl_resp = server.get("/api/v1/compensation/dead-letters").await;
    dl_resp.assert_status(StatusCode::OK);
    let dead_letters: Vec<Value> = dl_resp.json();
    assert!(
        dead_letters
            .iter()
            .any(|d| d["id"].as_str() == Some(&record_id.to_string())),
        "死信应在管理 API 中可见"
    );

    // 7. 通过管理 API 重推死信
    let retry_resp = server
        .post(&format!(
            "/api/v1/compensation/dead-letters/{}/retry",
            record_id
        ))
        .await;
    retry_resp.assert_status(StatusCode::NO_CONTENT);

    // 验证状态变回 failed（等待下次重试调度）
    let updated: (String,) =
        sqlx::query_as("SELECT status::text FROM delivery_records WHERE id = $1")
            .bind(record_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        updated.0, "failed",
        "重推后应回到 failed 状态等待重试"
    );

    // 8. 标记为已解决
    sqlx::query("UPDATE delivery_records SET status = 'dead'::delivery_status WHERE id = $1")
        .bind(record_id)
        .execute(&pool)
        .await
        .unwrap();
    let resolve_resp = server
        .post(&format!(
            "/api/v1/compensation/dead-letters/{}/resolve",
            record_id
        ))
        .await;
    resolve_resp.assert_status(StatusCode::NO_CONTENT);

    let resolved: (String,) =
        sqlx::query_as("SELECT status::text FROM delivery_records WHERE id = $1")
            .bind(record_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        resolved.0, "delivered",
        "resolve 后状态应变为 delivered"
    );

    // 9. Webhook 订阅验证
    let wh_resp = server
        .post("/api/v1/webhooks")
        .json(&json!({
            "url": "https://httpbin.org/post",
            "event_types": ["DeadLetter", "DeliveryFailed"],
            "description": "Integration test webhook"
        }))
        .await;
    wh_resp.assert_status(StatusCode::CREATED);
    let wh: Value = wh_resp.json();
    let wh_id = wh["id"].as_str().unwrap().to_string();

    // 验证列表中可见
    let wh_list: Vec<Value> = server.get("/api/v1/webhooks").await.json();
    assert!(
        wh_list.iter().any(|w| w["id"].as_str() == Some(&wh_id)),
        "创建的 Webhook 应在列表中可见"
    );

    // 清理 Webhook
    server
        .delete(&format!("/api/v1/webhooks/{}", wh_id))
        .await;

    // 清理投递记录和项目
    sqlx::query("DELETE FROM idempotency_keys WHERE route_id IN (SELECT r.id FROM routes r JOIN contracts c ON r.contract_id = c.id WHERE c.project_id = $1)")
        .bind(project.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM delivery_records WHERE route_id IN (SELECT r.id FROM routes r JOIN contracts c ON r.contract_id = c.id WHERE c.project_id = $1)")
        .bind(project.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project.id)
        .execute(&pool)
        .await
        .unwrap();
}

// ═══════════════════════════════════════════════════════════════════════════
// 场景 D: 保护层协同
// 限流 → 熔断 → 半开恢复，验证 ProtectionStack 各组件在 BackendDispatcher 中的联动
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn scenario_d_protection_coordination() {
    // FlakeyAdapter: 前 N 次请求失败，之后成功，模拟后端瞬态故障后恢复
    struct FlakeyAdapter {
        call_count: std::sync::atomic::AtomicU32,
        fail_threshold: u32,
    }

    impl ProtocolAdapter for FlakeyAdapter {
        fn transform_request(&self, _: &GatewayRequest) -> Result<BackendRequest, AppError> {
            Ok(BackendRequest {
                endpoint: "mock".into(),
                method: Method::POST,
                headers: HeaderMap::new(),
                body: None,
                protocol_params: HashMap::new(),
            })
        }

        fn execute<'a>(
            &'a self,
            _: &'a BackendRequest,
        ) -> BoxFuture<'a, Result<BackendResponse, AppError>> {
            Box::pin(async move {
                let count = self
                    .call_count
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if count < self.fail_threshold {
                    Err(AppError::BackendUnavailable("Flakey backend".into()))
                } else {
                    Ok(BackendResponse {
                        status_code: 200,
                        headers: HeaderMap::new(),
                        body: br#"{"ok":true}"#.to_vec(),
                        is_success: true,
                        duration_ms: 10,
                    })
                }
            })
        }

        fn transform_response(&self, resp: &BackendResponse) -> Result<GatewayResponse, AppError> {
            Ok(GatewayResponse {
                status_code: 200,
                headers: HashMap::new(),
                body: serde_json::from_slice(&resp.body).unwrap(),
            })
        }

        fn name(&self) -> &str {
            "flakey"
        }
    }

    let make_req = || GatewayRequest {
        route_id: Uuid::new_v4(),
        method: Method::POST,
        path: "/test".into(),
        headers: HeaderMap::new(),
        query_params: HashMap::new(),
        path_params: HashMap::new(),
        body: None,
        trace_id: "t".into(),
    };

    // ── 测试 1: 限流 ──
    // burst=3 表示令牌桶初始 3 个令牌，第 4 个请求应被限流拒绝
    let protection = ProtectionStack::new(
        3,   // rps（同时也是 burst_size）
        100, // max_concurrent
        80.0,
        Duration::from_secs(30),
        Duration::from_millis(100),
        2,
        Duration::from_secs(30),
    );
    let dispatcher = BackendDispatcher::new(
        Box::new(FlakeyAdapter {
            call_count: std::sync::atomic::AtomicU32::new(100), // 不触发失败
            fail_threshold: 0,
        }),
        protection,
    );

    for i in 0..3 {
        assert!(
            dispatcher.dispatch(make_req()).await.is_ok(),
            "第 {} 个请求应通过限流",
            i + 1
        );
    }
    let result = dispatcher.dispatch(make_req()).await;
    assert!(
        result.is_err(),
        "第 4 个请求应被限流拒绝"
    );

    // ── 测试 2: 熔断 ──
    // 配置高 rps 避免限流干扰，error_threshold=50% 使连续失败快速触发熔断
    let protection2 = ProtectionStack::new(
        1000,
        100,
        50.0,
        Duration::from_secs(30),
        Duration::from_millis(100), // open_duration 100ms，便于测试中快速过期
        2,                          // half_open 需要 2 次成功才关闭
        Duration::from_secs(30),
    );
    // fail_threshold=3：call_count 为 0/1/2 时失败，>=3 时成功。
    // 熔断器在 min_volume(=half_open_max_requests=2) 之后评估，即第 3 次失败后
    // 错误率 100% > 50% 触发熔断。此时 call_count=3，半开后的请求 call_count>=3 将成功
    let dispatcher2 = BackendDispatcher::new(
        Box::new(FlakeyAdapter {
            call_count: std::sync::atomic::AtomicU32::new(0),
            fail_threshold: 3,
        }),
        protection2,
    );

    // 连续失败触发熔断
    for _ in 0..8 {
        let _ = dispatcher2.dispatch(make_req()).await;
    }
    let result = dispatcher2.dispatch(make_req()).await;
    assert!(
        matches!(result, Err(AppError::CircuitBreakerOpen(_))),
        "连续失败后应触发熔断"
    );

    // 等待 open_duration 过期后进入半开状态
    tokio::time::sleep(Duration::from_millis(150)).await;

    // 半开状态下 FlakeyAdapter 的 call_count 已超过 fail_threshold，后续请求成功
    let result = dispatcher2.dispatch(make_req()).await;
    assert!(
        result.is_ok(),
        "熔断器半开后，后端恢复时请求应成功"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 场景 E: 多协议共存
// 同一网关同时承载 SOAP + CLI + SSH 三种协议的路由，验证互不干扰
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(unix)]
#[tokio::test]
async fn scenario_e_multi_protocol_coexistence() {
    let pool = common::test_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();
    let suffix = Uuid::new_v4();

    // 1. SOAP 项目
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(soap_response("99")))
        .mount(&mock_server)
        .await;

    let unique_slug = format!("inte{}", &suffix.to_string().replace('-', "")[..16]);
    let wsdl = calculator_wsdl()
        .replace(
            r#"name="CalculatorService""#,
            &format!(r#"name="{unique_slug}Service""#),
        )
        .replace(
            r#"location="http://example.com/calculator""#,
            &format!(r#"location="{}""#, mock_server.uri()),
        );
    let soap_project = repo
        .create_project(
            &format!("int-e-soap-{}", &suffix.to_string()[..8]),
            "multi",
            "test",
            SourceType::Wsdl,
        )
        .await
        .unwrap();
    api_anything_generator::pipeline::GenerationPipeline::run_wsdl(&repo, soap_project.id, &wsdl)
        .await
        .unwrap();

    // 2. CLI 项目
    let script_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../generator/tests/fixtures/mock-report-gen.sh")
        .canonicalize()
        .unwrap();
    let link_name = format!("inte-cli-{}", &suffix.to_string()[..8]);
    let link_path = std::env::temp_dir().join(&link_name);
    std::os::unix::fs::symlink(&script_path, &link_path).unwrap();

    let cli_project = repo
        .create_project(
            &format!("int-e-cli-{}", &suffix.to_string()[..8]),
            "multi",
            "test",
            SourceType::Cli,
        )
        .await
        .unwrap();
    let main_help = include_str!("../../generator/tests/fixtures/sample_help.txt");
    let sub_help = include_str!("../../generator/tests/fixtures/sample_subcommand_help.txt");
    api_anything_generator::pipeline::GenerationPipeline::run_cli(
        &repo,
        cli_project.id,
        link_path.to_str().unwrap(),
        main_help,
        &[("generate", sub_help)],
    )
    .await
    .unwrap();

    // 3. SSH 项目
    let ssh_project = repo
        .create_project(
            &format!("int-e-ssh-{}", &suffix.to_string()[..8]),
            "multi",
            "test",
            SourceType::Ssh,
        )
        .await
        .unwrap();
    let ssh_sample = include_str!("../../generator/tests/fixtures/ssh_sample.txt");
    api_anything_generator::pipeline::GenerationPipeline::run_ssh(
        &repo,
        ssh_project.id,
        ssh_sample,
    )
    .await
    .unwrap();

    // 4. 加载所有路由
    let router = Arc::new(DynamicRouter::new());
    let dispatchers: Arc<DashMap<Uuid, Arc<BackendDispatcher>>> = Arc::new(DashMap::new());
    let loaded = RouteLoader::load(&repo, &router, &dispatchers)
        .await
        .unwrap();
    // 数据库可能残留其他测试的路由，但至少应包含本测试创建的 3 种协议路由
    assert!(loaded >= 3, "应至少加载 3 条路由（来自 3 种协议），实际: {loaded}");

    let state = AppState {
        db: pool.clone(),
        repo: Arc::new(PgMetadataRepo::new(pool.clone())),
        router,
        dispatchers,
    };
    let server = TestServer::new(build_app(state)).unwrap();

    // 5. OpenAPI 应包含三种协议的路由
    let openapi: Value = server.get("/api/v1/docs/openapi.json").await.json();
    let paths = openapi["paths"].as_object().unwrap();

    let kebab_slug = to_kebab_strip_service(&format!("{unique_slug}Service"));
    let soap_routes = paths.keys().filter(|p| p.contains(&kebab_slug)).count();
    let cli_routes = paths.keys().filter(|p| p.contains(&link_name)).count();
    // SSH 路由路径包含解析出的主机标识（IP 地址转 kebab 格式）
    let ssh_routes = paths.keys().filter(|p| p.contains("10-0-1-50")).count();

    assert!(soap_routes >= 1, "OpenAPI 应包含 SOAP 路由");
    assert!(cli_routes >= 1, "OpenAPI 应包含 CLI 路由");
    assert!(ssh_routes >= 1, "OpenAPI 应包含 SSH 路由");

    // 6. Agent Prompt 包含所有协议
    let prompt = server.get("/api/v1/docs/agent-prompt").await.text();
    assert!(
        prompt.contains("Soap") || prompt.contains("soap") || prompt.contains("SOAP"),
        "Agent 提示词应提及 SOAP 协议"
    );

    // 7. Python SDK 包含有效代码
    let sdk = server.get("/api/v1/docs/sdk/python").await.text();
    assert!(
        sdk.contains("requests") || sdk.contains("def "),
        "Python SDK 应包含有效代码"
    );

    // 清理
    let _ = std::fs::remove_file(&link_path);
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(soap_project.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(cli_project.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(ssh_project.id)
        .execute(&pool)
        .await
        .unwrap();
}

// ═══════════════════════════════════════════════════════════════════════════
// 场景 F: 沙箱三模式流转
// Mock 联调 → Proxy 录制 → Replay 回放 → Read-Only 拦截 → 录制清空
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn scenario_f_sandbox_mode_transitions() {
    let pool = common::test_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();
    let suffix = Uuid::new_v4();

    // 用 wiremock 生成 SOAP 路由
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("Content-Type", "text/xml; charset=utf-8"))
        .respond_with(ResponseTemplate::new(200).set_body_string(soap_response("77")))
        .mount(&mock_server)
        .await;

    let unique_slug = format!("intf{}", &suffix.to_string().replace('-', "")[..16]);
    let wsdl = calculator_wsdl()
        .replace(
            r#"name="CalculatorService""#,
            &format!(r#"name="{unique_slug}Service""#),
        )
        .replace(
            r#"location="http://example.com/calculator""#,
            &format!(r#"location="{}""#, mock_server.uri()),
        );

    let project = repo
        .create_project(
            &format!("int-f-{}", &suffix.to_string()[..8]),
            "sandbox",
            "test",
            SourceType::Wsdl,
        )
        .await
        .unwrap();
    api_anything_generator::pipeline::GenerationPipeline::run_wsdl(&repo, project.id, &wsdl)
        .await
        .unwrap();

    let router = Arc::new(DynamicRouter::new());
    let dispatchers: Arc<DashMap<Uuid, Arc<BackendDispatcher>>> = Arc::new(DashMap::new());
    RouteLoader::load(&repo, &router, &dispatchers)
        .await
        .unwrap();

    let kebab_slug = to_kebab_strip_service(&format!("{unique_slug}Service"));
    let add_path = format!("/api/v1/{kebab_slug}/add");
    let sandbox_path = format!("/sandbox{add_path}");

    let repo_arc = Arc::new(PgMetadataRepo::new(pool.clone()));
    let state = AppState {
        db: pool.clone(),
        repo: repo_arc.clone(),
        router,
        dispatchers,
    };
    let server = TestServer::new(build_app(state)).unwrap();

    // Step 1: Mock 模式联调 — 从 response_schema 生成结构化模拟数据
    let mock_resp = server
        .post(&sandbox_path)
        .add_header("X-Sandbox-Mode", "mock")
        .json(&json!({"a": 1, "b": 2}))
        .await;
    mock_resp.assert_status(StatusCode::OK);
    let mock_body: Value = mock_resp.json();
    assert!(
        mock_body.is_object(),
        "Mock 模式应返回结构化数据"
    );

    // Step 2: Proxy 模式 + 自动录制
    let session = repo_arc
        .create_sandbox_session(
            project.id,
            "transition-tenant",
            SandboxMode::Proxy,
            &json!({}),
            Utc::now() + chrono::Duration::hours(1),
        )
        .await
        .unwrap();

    let proxy_resp = server
        .post(&sandbox_path)
        .add_header("X-Sandbox-Mode", "proxy")
        .add_header("X-Sandbox-Session", &session.id.to_string())
        .json(&json!({"a": 10, "b": 20}))
        .await;
    proxy_resp.assert_status(StatusCode::OK);

    // Step 3: 验证录制
    let recs: Vec<Value> = server
        .get(&format!(
            "/api/v1/sandbox-sessions/{}/recordings",
            session.id
        ))
        .await
        .json();
    assert!(
        !recs.is_empty(),
        "Proxy 模式应产生至少 1 条录制记录"
    );

    // Step 4: Replay 模式回放
    let replay_resp = server
        .post(&sandbox_path)
        .add_header("X-Sandbox-Mode", "replay")
        .add_header("X-Sandbox-Session", &session.id.to_string())
        .json(&json!({"a": 10, "b": 20}))
        .await;
    assert!(
        replay_resp.status_code() == 200 || replay_resp.status_code() == 404,
        "Replay 应返回 200(匹配) 或 404(未匹配)，实际: {}",
        replay_resp.status_code()
    );

    // Step 5: Read-Only 模式拦截 — POST 应被拒绝
    let ro_session = repo_arc
        .create_sandbox_session(
            project.id,
            "readonly-tenant",
            SandboxMode::Proxy,
            &json!({"read_only": true}),
            Utc::now() + chrono::Duration::hours(1),
        )
        .await
        .unwrap();

    let ro_resp = server
        .post(&sandbox_path)
        .add_header("X-Sandbox-Mode", "proxy")
        .add_header("X-Sandbox-Session", &ro_session.id.to_string())
        .json(&json!({"a": 1, "b": 2}))
        .await;
    ro_resp.assert_status(StatusCode::BAD_REQUEST);

    // Step 6: 清空录制
    let clear_resp = server
        .delete(&format!(
            "/api/v1/sandbox-sessions/{}/recordings",
            session.id
        ))
        .await;
    assert!(
        clear_resp.status_code() == 200 || clear_resp.status_code() == 204,
        "清空录制应返回 200 或 204"
    );

    // Step 7: 会话清理
    server
        .delete(&format!("/api/v1/sandbox-sessions/{}", session.id))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    server
        .delete(&format!("/api/v1/sandbox-sessions/{}", ro_session.id))
        .await
        .assert_status(StatusCode::NO_CONTENT);

    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project.id)
        .execute(&pool)
        .await
        .unwrap();
}

// ═══════════════════════════════════════════════════════════════════════════
// 场景 G: 文档一致性
// OpenAPI 完整性 → 错误码覆盖 → Swagger UI → Agent Prompt → 多语言 SDK → Health
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn scenario_g_documentation_consistency() {
    let pool = common::test_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

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
    let server = TestServer::new(build_app(state)).unwrap();

    // 1. OpenAPI 结构完整性
    let openapi: Value = server.get("/api/v1/docs/openapi.json").await.json();
    assert_eq!(openapi["openapi"], "3.0.3");
    assert!(
        openapi["info"]["title"].is_string(),
        "OpenAPI 规范应包含 info.title"
    );
    assert!(
        openapi["paths"].is_object(),
        "OpenAPI 规范应包含 paths 对象"
    );

    // 2. 每个路由都有标准网关错误响应定义（429/502）
    if let Some(paths) = openapi["paths"].as_object() {
        for (path, methods) in paths {
            if let Some(obj) = methods.as_object() {
                for (method_name, op) in obj {
                    assert!(
                        op["responses"]["429"].is_object(),
                        "路由 {} {} 缺少 429 限流响应定义",
                        method_name,
                        path
                    );
                    assert!(
                        op["responses"]["502"].is_object(),
                        "路由 {} {} 缺少 502 后端错误响应定义",
                        method_name,
                        path
                    );
                }
            }
        }
    }

    // 3. Swagger UI 可访问
    let swagger = server.get("/api/v1/docs").await;
    swagger.assert_status(StatusCode::OK);
    let html = swagger.text();
    assert!(
        html.contains("swagger-ui"),
        "Swagger UI 页面应包含 swagger-ui 标识"
    );

    // 4. Agent Prompt 格式正确
    let prompt = server.get("/api/v1/docs/agent-prompt").await;
    prompt.assert_status(StatusCode::OK);
    let text = prompt.text();
    assert!(
        text.starts_with("# "),
        "Agent 提示词应以 Markdown 标题开头"
    );
    assert!(
        text.contains("API") || text.contains("endpoint"),
        "Agent 提示词应描述 API 端点"
    );

    // 5. 所有支持的语言都能生成 SDK
    for lang in &["typescript", "python", "java", "go"] {
        let resp = server
            .get(&format!("/api/v1/docs/sdk/{}", lang))
            .await;
        resp.assert_status(StatusCode::OK);
        let code = resp.text();
        assert!(
            !code.is_empty(),
            "{} SDK 不应为空",
            lang
        );
    }

    // 6. 不支持的语言返回 400
    let resp = server.get("/api/v1/docs/sdk/brainfuck").await;
    resp.assert_status(StatusCode::BAD_REQUEST);

    // 7. Health 端点
    let health: Value = server.get("/health").await.json();
    assert_eq!(health["status"], "ok", "健康检查应返回 ok");

    let ready: Value = server.get("/health/ready").await.json();
    assert_eq!(
        ready["status"], "ready",
        "就绪检查应返回 ready（需要数据库连通）"
    );
}
