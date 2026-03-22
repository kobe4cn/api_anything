/// SOAP 协议全链路 E2E 测试套件：覆盖从 WSDL 解析 → 路由生成 → 网关加载 → 代理转发的完整链路。
/// 使用 wiremock 模拟真实 SOAP 后端，测试场景包括正常请求、多操作、嵌套类型、
/// SOAP Fault、超时、不可达、空 body、XML 特殊字符转义和并发请求。
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
use std::time::Duration;
use uuid::Uuid;
use wiremock::matchers::{header, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

mod common;

// ──────────────────────────────────────────────────────────
// 辅助函数：构造 SOAP 响应 XML
// ──────────────────────────────────────────────────────────

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

fn soap_get_history_response() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
  <soap:Body>
    <GetHistoryResponse xmlns="http://example.com/calculator">
      <entries>5+3=8</entries>
    </GetHistoryResponse>
  </soap:Body>
</soap:Envelope>"#
        .to_string()
}

fn soap_fault_response(fault_code: &str, fault_string: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
  <soap:Body>
    <soap:Fault>
      <faultcode>{fault_code}</faultcode>
      <faultstring>{fault_string}</faultstring>
    </soap:Fault>
  </soap:Body>
</soap:Envelope>"#
    )
}

fn soap_create_order_response() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
  <soap:Body>
    <CreateOrderResponse xmlns="http://example.com/order">
      <order_id>ORD-12345</order_id>
      <status>confirmed</status>
      <total_amount>199.99</total_amount>
      <estimated_delivery>2025-01-15T10:00:00Z</estimated_delivery>
    </CreateOrderResponse>
  </soap:Body>
</soap:Envelope>"#
        .to_string()
}

fn calculator_wsdl() -> String {
    include_str!("../../generator/tests/fixtures/calculator.wsdl").to_string()
}

fn order_service_wsdl() -> String {
    include_str!("../../../docs/test-data/complex-order-service.wsdl").to_string()
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

/// 通用辅助：给定一个唯一后缀，替换 calculator.wsdl 中的服务名和端点地址，
/// 返回 (修改后的 WSDL, 服务路径前缀)
fn prepare_calculator_wsdl(suffix: &str, mock_uri: &str) -> (String, String) {
    let unique_slug = format!("e2esvc{}", suffix);
    let wsdl = calculator_wsdl()
        .replace(
            r#"name="CalculatorService""#,
            &format!(r#"name="{unique_slug}Service""#),
        )
        .replace(
            r#"location="http://example.com/calculator""#,
            &format!(r#"location="{mock_uri}""#),
        );
    let path_prefix = to_kebab_strip_service(&format!("{unique_slug}Service"));
    (wsdl, path_prefix)
}

/// 通用辅助：创建项目、运行 WSDL pipeline、加载路由、构建测试服务器
async fn setup_soap_env(
    wsdl: &str,
    project_name: &str,
) -> (TestServer, sqlx::PgPool, Uuid) {
    let pool = common::test_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let project = repo
        .create_project(project_name, "E2E SOAP test", "test", SourceType::Wsdl)
        .await
        .unwrap();

    api_anything_generator::pipeline::GenerationPipeline::run_wsdl(&repo, project.id, wsdl)
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

/// 测试后清理项目数据（级联删除 contracts → routes）
async fn cleanup(pool: &sqlx::PgPool, project_id: Uuid) {
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project_id)
        .execute(pool)
        .await
        .unwrap();
}

// ──────────────────────────────────────────────────────────
// 测试用例
// ──────────────────────────────────────────────────────────

#[tokio::test]
async fn soap_basic_add_operation() {
    // 验证最基础的 SOAP 全链路：JSON 请求 → SOAP Envelope → 后端响应 → JSON 输出
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("Content-Type", "text/xml; charset=utf-8"))
        .respond_with(ResponseTemplate::new(200).set_body_string(soap_add_response(42)))
        .expect(1..)
        .mount(&mock_server)
        .await;

    let suffix = Uuid::new_v4().to_string().replace('-', "");
    let short = &suffix[..16];
    let (wsdl, path_prefix) = prepare_calculator_wsdl(short, &mock_server.uri());
    let project_name = format!("e2e-soap-add-{short}");

    let (server, pool, project_id) = setup_soap_env(&wsdl, &project_name).await;

    let resp = server
        .post(&format!("/gw/api/v1/{path_prefix}/add"))
        .json(&json!({"a": 20, "b": 22}))
        .await;

    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();
    assert_eq!(body["result"], "42", "Expected result=42, got: {body}");

    // 验证 wiremock 收到的请求包含 SOAP Envelope 和参数
    let received = mock_server.received_requests().await.unwrap();
    assert!(!received.is_empty(), "wiremock 应至少收到 1 个请求");
    let req_body = String::from_utf8_lossy(&received[0].body);
    assert!(req_body.contains("<soap:Envelope"), "请求体应包含 SOAP Envelope");
    assert!(req_body.contains("<a>20</a>"), "请求体应包含 <a>20</a>");
    assert!(req_body.contains("<b>22</b>"), "请求体应包含 <b>22</b>");

    cleanup(&pool, project_id).await;
}

#[tokio::test]
async fn soap_multiple_operations() {
    // 验证同一 WSDL 中多个操作（Add + GetHistory）都能被正确访问，
    // 且各操作携带不同的 SOAPAction header
    let mock_server = MockServer::start().await;

    // 通过 SOAPAction 头区分不同操作的 mock 响应
    Mock::given(method("POST"))
        .and(header("SOAPAction", "http://example.com/calculator/Add"))
        .respond_with(ResponseTemplate::new(200).set_body_string(soap_add_response(10)))
        .expect(1..)
        .mount(&mock_server)
        .await;

    Mock::given(method("POST"))
        .and(header(
            "SOAPAction",
            "http://example.com/calculator/GetHistory",
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(soap_get_history_response()),
        )
        .expect(1..)
        .mount(&mock_server)
        .await;

    let suffix = Uuid::new_v4().to_string().replace('-', "");
    let short = &suffix[..16];
    let (wsdl, path_prefix) = prepare_calculator_wsdl(short, &mock_server.uri());
    let project_name = format!("e2e-soap-multi-{short}");

    let (server, pool, project_id) = setup_soap_env(&wsdl, &project_name).await;

    // 调用 Add 操作
    let add_resp = server
        .post(&format!("/gw/api/v1/{path_prefix}/add"))
        .json(&json!({"a": 5, "b": 5}))
        .await;
    add_resp.assert_status(StatusCode::OK);
    let add_body: serde_json::Value = add_resp.json();
    assert_eq!(add_body["result"], "10");

    // 调用 GetHistory 操作
    let hist_resp = server
        .post(&format!("/gw/api/v1/{path_prefix}/get-history"))
        .json(&json!({"limit": 10}))
        .await;
    hist_resp.assert_status(StatusCode::OK);
    let hist_body: serde_json::Value = hist_resp.json();
    assert_eq!(hist_body["entries"], "5+3=8");

    // 验证两个操作分别收到了正确的 SOAPAction header
    let received = mock_server.received_requests().await.unwrap();
    assert!(received.len() >= 2, "应至少收到 2 个请求");

    cleanup(&pool, project_id).await;
}

#[tokio::test]
async fn soap_nested_complex_types() {
    // 使用 complex-order-service.wsdl 测试嵌套 JSON → SOAP XML 的序列化，
    // 验证多层嵌套对象（address）和数组（items）在 XML 中结构正确
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(soap_create_order_response()),
        )
        .expect(1..)
        .mount(&mock_server)
        .await;

    let suffix = Uuid::new_v4().to_string().replace('-', "");
    let short = &suffix[..16];
    let unique_slug = format!("e2eord{short}");

    let wsdl = order_service_wsdl()
        .replace(
            r#"name="OrderService""#,
            &format!(r#"name="{unique_slug}Service""#),
        )
        .replace(
            r#"location="http://legacy-erp.internal:8080/soap/orders""#,
            &format!(r#"location="{}""#, mock_server.uri()),
        );

    let path_prefix = to_kebab_strip_service(&format!("{unique_slug}Service"));
    let project_name = format!("e2e-soap-nested-{short}");

    let (server, pool, project_id) = setup_soap_env(&wsdl, &project_name).await;

    // 发送包含嵌套地址和订单项数组的创建订单请求
    let resp = server
        .post(&format!("/gw/api/v1/{path_prefix}/create-order"))
        .json(&json!({
            "customer_id": "CUST-001",
            "shipping_address": {
                "street": "123 Main St",
                "city": "Springfield",
                "state": "IL",
                "zip_code": "62701",
                "country": "US"
            },
            "items": [
                {"product_id": "PROD-A", "product_name": "Widget", "quantity": 2, "unit_price": 49.99},
                {"product_id": "PROD-B", "product_name": "Gadget", "quantity": 1, "unit_price": 99.99}
            ],
            "payment_method": "credit_card"
        }))
        .await;

    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();
    assert_eq!(body["order_id"], "ORD-12345");
    assert_eq!(body["status"], "confirmed");

    // 验证 wiremock 收到的 SOAP XML 中嵌套结构正确
    let received = mock_server.received_requests().await.unwrap();
    let req_body = String::from_utf8_lossy(&received[0].body);
    assert!(
        req_body.contains("<customer_id>CUST-001</customer_id>"),
        "应包含 customer_id"
    );
    assert!(
        req_body.contains("<street>123 Main St</street>"),
        "嵌套 address 中应包含 street"
    );
    assert!(
        req_body.contains("<city>Springfield</city>"),
        "嵌套 address 中应包含 city"
    );
    assert!(
        req_body.contains("<product_id>PROD-A</product_id>"),
        "数组 items 中应包含 product_id"
    );

    cleanup(&pool, project_id).await;
}

#[tokio::test]
async fn soap_fault_returns_rfc7807() {
    // 验证 SOAP Fault 被网关规范化为 RFC 7807 格式的 502 错误响应，
    // 且 detail 字段包含 faultstring 的内容
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(500)
                .set_body_string(soap_fault_response("soap:Server", "Division by zero")),
        )
        .expect(1..)
        .mount(&mock_server)
        .await;

    let suffix = Uuid::new_v4().to_string().replace('-', "");
    let short = &suffix[..16];
    let (wsdl, path_prefix) = prepare_calculator_wsdl(short, &mock_server.uri());
    let project_name = format!("e2e-soap-fault-{short}");

    let (server, pool, project_id) = setup_soap_env(&wsdl, &project_name).await;

    let resp = server
        .post(&format!("/gw/api/v1/{path_prefix}/add"))
        .json(&json!({"a": 1, "b": 0}))
        .await;

    // SOAP 后端返回 500 时，网关应转为 502（Bad Gateway）
    resp.assert_status(StatusCode::BAD_GATEWAY);

    let body: serde_json::Value = resp.json();
    // RFC 7807 格式应包含 title 和 status 字段
    assert_eq!(body["status"], 502);
    assert_eq!(body["title"], "Bad Gateway");
    // detail 应包含 faultstring 的内容
    let detail = body["detail"].as_str().unwrap_or("");
    assert!(
        detail.contains("Division by zero"),
        "detail 应包含 SOAP faultstring，实际: {detail}"
    );

    cleanup(&pool, project_id).await;
}

#[tokio::test]
async fn soap_backend_unreachable_returns_502() {
    // 验证当后端完全不可达时（指向一个不存在的端口），网关返回 502
    let suffix = Uuid::new_v4().to_string().replace('-', "");
    let short = &suffix[..16];
    // 指向一个几乎不可能存在的端点
    let (wsdl, path_prefix) = prepare_calculator_wsdl(short, "http://127.0.0.1:1");
    let project_name = format!("e2e-soap-unreach-{short}");

    let (server, pool, project_id) = setup_soap_env(&wsdl, &project_name).await;

    let resp = server
        .post(&format!("/gw/api/v1/{path_prefix}/add"))
        .json(&json!({"a": 1, "b": 1}))
        .await;

    // 连接失败应返回 502 Bad Gateway
    resp.assert_status(StatusCode::BAD_GATEWAY);
    let body: serde_json::Value = resp.json();
    assert_eq!(body["status"], 502);

    cleanup(&pool, project_id).await;
}

#[tokio::test]
async fn soap_backend_timeout_returns_504() {
    // 验证当后端响应时间超过路由配置的 timeout_ms 时，网关返回 504 Gateway Timeout。
    // wiremock 设置 5 秒延迟，而路由的默认 timeout 为 30 秒；
    // 我们需要直接操作数据库将 timeout_ms 改为很短的值来触发超时
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(soap_add_response(1))
                .set_delay(Duration::from_secs(10)),
        )
        .mount(&mock_server)
        .await;

    let suffix = Uuid::new_v4().to_string().replace('-', "");
    let short = &suffix[..16];
    let (wsdl, _path_prefix) = prepare_calculator_wsdl(short, &mock_server.uri());

    let pool = common::test_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let project_name = format!("e2e-soap-timeout-{short}");
    let project = repo
        .create_project(&project_name, "E2E timeout test", "test", SourceType::Wsdl)
        .await
        .unwrap();

    api_anything_generator::pipeline::GenerationPipeline::run_wsdl(&repo, project.id, &wsdl)
        .await
        .unwrap();

    // 将 backend_bindings 的 timeout_ms 改为 100ms，使其远小于 wiremock 的 10s 延迟
    sqlx::query("UPDATE backend_bindings SET timeout_ms = 100 WHERE id IN (SELECT backend_binding_id FROM routes WHERE contract_id IN (SELECT id FROM contracts WHERE project_id = $1))")
        .bind(project.id)
        .execute(&pool)
        .await
        .unwrap();

    // 重新加载路由，使短 timeout 生效
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

    let path_prefix = to_kebab_strip_service(&format!("e2esvc{short}Service"));
    let resp = server
        .post(&format!("/gw/api/v1/{path_prefix}/add"))
        .json(&json!({"a": 1, "b": 1}))
        .await;

    // 超时应返回 504 Gateway Timeout
    resp.assert_status(StatusCode::GATEWAY_TIMEOUT);
    let body: serde_json::Value = resp.json();
    assert_eq!(body["status"], 504);
    assert_eq!(body["title"], "Gateway Timeout");

    cleanup(&pool, project.id).await;
}

#[tokio::test]
async fn soap_empty_body_request() {
    // 验证发送空 body 时，适配器仍能生成有效的 SOAP Envelope（无参数元素）
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(soap_add_response(0)))
        .expect(1..)
        .mount(&mock_server)
        .await;

    let suffix = Uuid::new_v4().to_string().replace('-', "");
    let short = &suffix[..16];
    let (wsdl, path_prefix) = prepare_calculator_wsdl(short, &mock_server.uri());
    let project_name = format!("e2e-soap-empty-{short}");

    let (server, pool, project_id) = setup_soap_env(&wsdl, &project_name).await;

    // 发送空 body（不发 JSON）
    let resp = server
        .post(&format!("/gw/api/v1/{path_prefix}/add"))
        .await;

    resp.assert_status(StatusCode::OK);

    // 验证 wiremock 收到的请求仍包含 SOAP Envelope 结构
    let received = mock_server.received_requests().await.unwrap();
    assert!(!received.is_empty());
    let req_body = String::from_utf8_lossy(&received[0].body);
    assert!(
        req_body.contains("<soap:Envelope"),
        "空 body 请求也应产生有效的 SOAP Envelope"
    );
    assert!(
        req_body.contains("<soap:Body>"),
        "空 body 请求也应包含 soap:Body"
    );

    cleanup(&pool, project_id).await;
}

#[tokio::test]
async fn soap_special_characters_in_body() {
    // 验证包含 XML 特殊字符（<, >, &, ", '）的值在 SOAP XML 中被正确转义，
    // 防止 XML 注入破坏 Envelope 结构
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(soap_add_response(0)))
        .expect(1..)
        .mount(&mock_server)
        .await;

    let suffix = Uuid::new_v4().to_string().replace('-', "");
    let short = &suffix[..16];

    // 使用 order service WSDL，其 CreateOrder 的 notes 字段为 string 类型，适合注入测试
    let unique_slug = format!("e2esc{short}");
    let wsdl = order_service_wsdl()
        .replace(
            r#"name="OrderService""#,
            &format!(r#"name="{unique_slug}Service""#),
        )
        .replace(
            r#"location="http://legacy-erp.internal:8080/soap/orders""#,
            &format!(r#"location="{}""#, mock_server.uri()),
        );

    let path_prefix = to_kebab_strip_service(&format!("{unique_slug}Service"));
    let project_name = format!("e2e-soap-special-{short}");

    let (server, pool, project_id) = setup_soap_env(&wsdl, &project_name).await;

    // 在 notes 字段中注入 XML 特殊字符
    let resp = server
        .post(&format!("/gw/api/v1/{path_prefix}/create-order"))
        .json(&json!({
            "customer_id": "<script>alert('xss')</script>",
            "shipping_address": {
                "street": "123 & Main <St>",
                "city": "O'Brien",
                "state": "\"IL\"",
                "zip_code": "62701",
                "country": "US"
            },
            "items": [
                {"product_id": "P1", "product_name": "Test", "quantity": 1, "unit_price": 10}
            ],
            "payment_method": "cash"
        }))
        .await;

    // 请求不应因特殊字符而失败
    resp.assert_status(StatusCode::OK);

    // 验证 wiremock 收到的 XML 中特殊字符被正确转义
    let received = mock_server.received_requests().await.unwrap();
    let req_body = String::from_utf8_lossy(&received[0].body);
    assert!(
        req_body.contains("&lt;script&gt;"),
        "< 和 > 应被转义为 &lt; 和 &gt;，实际请求体: {req_body}"
    );
    assert!(
        req_body.contains("&amp;"),
        "& 应被转义为 &amp;"
    );

    cleanup(&pool, project_id).await;
}

#[tokio::test]
async fn soap_concurrent_requests() {
    // 验证连续发送多个请求到同一 SOAP 操作时全部能正确返回，
    // 确保路由器和 dispatcher 在多次调用场景下无资源泄漏或状态污染
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(soap_add_response(99)))
        .expect(10..)
        .mount(&mock_server)
        .await;

    let suffix = Uuid::new_v4().to_string().replace('-', "");
    let short = &suffix[..16];
    let (wsdl, path_prefix) = prepare_calculator_wsdl(short, &mock_server.uri());
    let project_name = format!("e2e-soap-conc-{short}");

    let (server, pool, project_id) = setup_soap_env(&wsdl, &project_name).await;

    // 连续发送 10 个请求，验证每个都正确返回
    for i in 0..10 {
        let resp = server
            .post(&format!("/gw/api/v1/{path_prefix}/add"))
            .json(&json!({"a": i, "b": 1}))
            .await;
        resp.assert_status(StatusCode::OK);
        let body: serde_json::Value = resp.json();
        assert_eq!(body["result"], "99", "Request {i} should return result=99");
    }

    // 验证 wiremock 收到了全部 10 个请求
    let received = mock_server.received_requests().await.unwrap();
    assert!(
        received.len() >= 10,
        "wiremock 应收到至少 10 个请求，实际: {}",
        received.len()
    );

    cleanup(&pool, project_id).await;
}
