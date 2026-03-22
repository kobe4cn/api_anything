/// 补偿机制全面 E2E 测试 — 覆盖三种投递保障语义（AtMostOnce / AtLeastOnce / ExactlyOnce）、
/// 重试调度配置验证、死信管理 API 的完整 CRUD 生命周期。
/// 投递保障测试通过 WSDL 生成路由后手动更新 delivery_guarantee 字段，
/// 再经由 /gw/ 网关端点触发真实的请求/记录/幂等键流程
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

/// 搭建补偿测试环境：
/// 1. 生成 WSDL 路由并加载到网关
/// 2. 更新指定路由的 delivery_guarantee
/// 3. 返回测试服务器和关键标识
async fn setup_with_delivery_guarantee(
    guarantee: &str,
    mock_server: &MockServer,
) -> (TestServer, Arc<PgMetadataRepo>, sqlx::PgPool, Uuid, String, Uuid) {
    let pool = common::test_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let suffix = Uuid::new_v4();
    let unique_slug = format!("comp{}", &suffix.to_string().replace('-', "")[..16]);

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
            &format!("e2e-comp-{suffix}"),
            "E2E compensation test",
            "test",
            SourceType::Wsdl,
        )
        .await
        .unwrap();

    api_anything_generator::pipeline::GenerationPipeline::run_wsdl(&repo, project.id, &wsdl)
        .await
        .unwrap();

    // 查找 add 路由并更新其 delivery_guarantee
    let routes = repo.list_active_routes_with_bindings().await.unwrap();
    let kebab_slug = to_kebab_strip_service(&format!("{unique_slug}Service"));
    let add_path = format!("/api/v1/{kebab_slug}/add");
    let target = routes
        .iter()
        .find(|r| r.path == add_path)
        .expect("Should find add route");
    let route_id = target.route_id;

    // 直接 SQL 更新 delivery_guarantee，因为 API 不提供修改此字段的端点
    sqlx::query("UPDATE routes SET delivery_guarantee = $1::delivery_guarantee WHERE id = $2")
        .bind(guarantee)
        .bind(route_id)
        .execute(&pool)
        .await
        .unwrap();

    let router = Arc::new(DynamicRouter::new());
    let dispatchers: Arc<DashMap<Uuid, Arc<BackendDispatcher>>> = Arc::new(DashMap::new());
    RouteLoader::load(&repo, &router, &dispatchers).await.unwrap();

    let repo_arc = Arc::new(repo);
    let state = AppState {
        db: pool.clone(),
        repo: repo_arc.clone(),
        router,
        dispatchers,
    };
    let server = TestServer::new(build_app(state)).unwrap();

    (server, repo_arc, pool, project.id, add_path, route_id)
}

async fn cleanup(pool: &sqlx::PgPool, project_id: Uuid) {
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project_id)
        .execute(pool)
        .await
        .unwrap();
}

// ---------------------------------------------------------------------------
// 投递保障
// ---------------------------------------------------------------------------

#[tokio::test]
async fn compensation_at_most_once_no_record() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("Content-Type", "text/xml; charset=utf-8"))
        .respond_with(ResponseTemplate::new(200).set_body_string(soap_add_response(8)))
        .mount(&mock_server)
        .await;

    let (server, _repo, pool, project_id, add_path, route_id) =
        setup_with_delivery_guarantee("at_most_once", &mock_server).await;

    let trace_id = format!("trace-amo-{}", Uuid::new_v4());
    let resp = server
        .post(&format!("/gw{add_path}"))
        .add_header("traceparent", &trace_id)
        .json(&json!({"a": 3, "b": 5}))
        .await;

    resp.assert_status(StatusCode::OK);

    // AtMostOnce 不应创建投递记录
    let records: Vec<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM delivery_records WHERE route_id = $1 AND trace_id = $2",
    )
    .bind(route_id)
    .bind(&trace_id)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert!(
        records.is_empty(),
        "AtMostOnce should not create delivery records, found {}",
        records.len()
    );

    cleanup(&pool, project_id).await;
}

#[tokio::test]
async fn compensation_at_least_once_creates_record() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("Content-Type", "text/xml; charset=utf-8"))
        .respond_with(ResponseTemplate::new(200).set_body_string(soap_add_response(12)))
        .mount(&mock_server)
        .await;

    let (server, _repo, pool, project_id, add_path, route_id) =
        setup_with_delivery_guarantee("at_least_once", &mock_server).await;

    let trace_id = format!("trace-alo-{}", Uuid::new_v4());
    let resp = server
        .post(&format!("/gw{add_path}"))
        .add_header("traceparent", &trace_id)
        .json(&json!({"a": 5, "b": 7}))
        .await;

    resp.assert_status(StatusCode::OK);

    // AtLeastOnce 成功后应有一条 status=delivered 的投递记录
    let record: (String,) = sqlx::query_as(
        "SELECT status::text FROM delivery_records WHERE route_id = $1 AND trace_id = $2",
    )
    .bind(route_id)
    .bind(&trace_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(
        record.0, "delivered",
        "AtLeastOnce success should be marked as delivered"
    );

    cleanup(&pool, project_id).await;
}

#[tokio::test]
async fn compensation_at_least_once_failure_creates_failed_record() {
    // 后端不响应 → 超时/失败
    let mock_server = MockServer::start().await;
    // 不挂载任何 mock → wiremock 返回 404，触发 BackendError

    let (server, _repo, pool, project_id, add_path, route_id) =
        setup_with_delivery_guarantee("at_least_once", &mock_server).await;

    let trace_id = format!("trace-alo-fail-{}", Uuid::new_v4());
    let resp = server
        .post(&format!("/gw{add_path}"))
        .add_header("traceparent", &trace_id)
        .json(&json!({"a": 1, "b": 2}))
        .await;

    // 后端返回 404，ErrorNormalizer 将其转为 BackendError(status=404)，
    // IntoResponse 将 BackendError 映射为 HTTP 502 Bad Gateway
    resp.assert_status(StatusCode::BAD_GATEWAY);

    // 应有一条 status=failed 且 next_retry_at 非空的投递记录
    let record: (String, Option<chrono::DateTime<chrono::Utc>>) = sqlx::query_as(
        "SELECT status::text, next_retry_at FROM delivery_records WHERE route_id = $1 AND trace_id = $2",
    )
    .bind(route_id)
    .bind(&trace_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(record.0, "failed", "Failed delivery should be marked as failed");
    assert!(
        record.1.is_some(),
        "Failed delivery should have next_retry_at set"
    );

    cleanup(&pool, project_id).await;
}

#[tokio::test]
async fn compensation_exactly_once_requires_idempotency_key() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(soap_add_response(0)))
        .mount(&mock_server)
        .await;

    let (server, _repo, pool, project_id, add_path, _route_id) =
        setup_with_delivery_guarantee("exactly_once", &mock_server).await;

    // 不提供 Idempotency-Key → 400
    let resp = server
        .post(&format!("/gw{add_path}"))
        .json(&json!({"a": 1, "b": 2}))
        .await;

    resp.assert_status(StatusCode::BAD_REQUEST);

    cleanup(&pool, project_id).await;
}

#[tokio::test]
async fn compensation_exactly_once_with_key() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("Content-Type", "text/xml; charset=utf-8"))
        .respond_with(ResponseTemplate::new(200).set_body_string(soap_add_response(20)))
        .mount(&mock_server)
        .await;

    let (server, _repo, pool, project_id, add_path, _route_id) =
        setup_with_delivery_guarantee("exactly_once", &mock_server).await;

    let idem_key = format!("idem-{}", Uuid::new_v4());
    let resp = server
        .post(&format!("/gw{add_path}"))
        .add_header("Idempotency-Key", &idem_key)
        .json(&json!({"a": 10, "b": 10}))
        .await;

    resp.assert_status(StatusCode::OK);

    // 验证 idempotency_keys 表有记录且 status=delivered
    let record: (String,) = sqlx::query_as(
        "SELECT status FROM idempotency_keys WHERE idempotency_key = $1",
    )
    .bind(&idem_key)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(
        record.0, "delivered",
        "Idempotency key should be marked delivered after success"
    );

    cleanup(&pool, project_id).await;
}

#[tokio::test]
async fn compensation_exactly_once_duplicate_returns_already_delivered() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("Content-Type", "text/xml; charset=utf-8"))
        .respond_with(ResponseTemplate::new(200).set_body_string(soap_add_response(30)))
        .mount(&mock_server)
        .await;

    let (server, _repo, pool, project_id, add_path, _route_id) =
        setup_with_delivery_guarantee("exactly_once", &mock_server).await;

    let idem_key = format!("idem-dup-{}", Uuid::new_v4());

    // 第一次请求
    let resp1 = server
        .post(&format!("/gw{add_path}"))
        .add_header("Idempotency-Key", &idem_key)
        .json(&json!({"a": 15, "b": 15}))
        .await;
    resp1.assert_status(StatusCode::OK);

    // 第二次相同 key → 应返回 200 {"status":"already_delivered"}
    let resp2 = server
        .post(&format!("/gw{add_path}"))
        .add_header("Idempotency-Key", &idem_key)
        .json(&json!({"a": 15, "b": 15}))
        .await;
    resp2.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp2.json();
    assert_eq!(
        body["status"], "already_delivered",
        "Duplicate key should return already_delivered, got: {body}"
    );

    cleanup(&pool, project_id).await;
}

// ---------------------------------------------------------------------------
// 重试调度配置
// ---------------------------------------------------------------------------

#[test]
fn compensation_retry_config_exponential_backoff() {
    use api_anything_compensation::config::RetryConfig;
    let config = RetryConfig::default();
    // 验证 §6.2 规格：1s → 5s → 30s → 5min → 30min
    assert_eq!(config.delay_for_attempt(0), Duration::from_secs(1));
    assert_eq!(config.delay_for_attempt(1), Duration::from_secs(5));
    assert_eq!(config.delay_for_attempt(2), Duration::from_secs(30));
    assert_eq!(config.delay_for_attempt(3), Duration::from_secs(300));
    assert_eq!(config.delay_for_attempt(4), Duration::from_secs(1800));
    // 超出预设序列封顶到最后一项
    assert_eq!(config.delay_for_attempt(99), Duration::from_secs(1800));
}

// ---------------------------------------------------------------------------
// 死信管理 API
// ---------------------------------------------------------------------------

#[tokio::test]
async fn compensation_dead_letter_list_empty() {
    let server = common::test_server().await;
    let resp = server.get("/api/v1/compensation/dead-letters").await;
    resp.assert_status(StatusCode::OK);
    // 返回数组（可能为空或包含之前测试残留）；
    // 反序列化为 Vec 即验证响应是合法 JSON 数组，无需检查长度
    let _body: Vec<serde_json::Value> = resp.json();
}

#[tokio::test]
async fn compensation_dead_letter_retry_nonexistent() {
    let server = common::test_server().await;
    let resp = server
        .post("/api/v1/compensation/dead-letters/00000000-0000-0000-0000-000000000000/retry")
        .await;
    resp.assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn compensation_dead_letter_resolve_nonexistent() {
    let server = common::test_server().await;
    let resp = server
        .post("/api/v1/compensation/dead-letters/00000000-0000-0000-0000-000000000000/resolve")
        .await;
    resp.assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn compensation_dead_letter_full_lifecycle() {
    let pool = common::test_pool().await;
    let server = common::test_server().await;

    let route_id = Uuid::new_v4();

    // 1. 手动创建一条 status=dead 的 delivery_record
    let record_id1: (Uuid,) = sqlx::query_as(
        r#"INSERT INTO delivery_records (route_id, trace_id, request_payload, status)
           VALUES ($1, $2, $3, 'dead'::delivery_status)
           RETURNING id"#,
    )
    .bind(route_id)
    .bind(format!("trace-dead-1-{}", Uuid::new_v4()))
    .bind(json!({"test": "dead-letter-lifecycle"}))
    .fetch_one(&pool)
    .await
    .unwrap();

    // 2. GET dead-letters 应能看到此记录
    let list_resp = server.get("/api/v1/compensation/dead-letters").await;
    list_resp.assert_status(StatusCode::OK);
    let dead_letters: Vec<serde_json::Value> = list_resp.json();
    assert!(
        dead_letters
            .iter()
            .any(|d| d["id"].as_str() == Some(&record_id1.0.to_string())),
        "Dead letter should be visible in list"
    );

    // 3. POST retry → 状态应变为 failed
    let retry_resp = server
        .post(&format!(
            "/api/v1/compensation/dead-letters/{}/retry",
            record_id1.0
        ))
        .await;
    retry_resp.assert_status(StatusCode::NO_CONTENT);

    let status_after_retry: (String,) = sqlx::query_as(
        "SELECT status::text FROM delivery_records WHERE id = $1",
    )
    .bind(record_id1.0)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        status_after_retry.0, "failed",
        "After retry, dead letter should be reset to failed"
    );

    // 4. 创建另一条 dead 记录 → resolve → 验证 delivered
    let record_id2: (Uuid,) = sqlx::query_as(
        r#"INSERT INTO delivery_records (route_id, trace_id, request_payload, status)
           VALUES ($1, $2, $3, 'dead'::delivery_status)
           RETURNING id"#,
    )
    .bind(route_id)
    .bind(format!("trace-dead-2-{}", Uuid::new_v4()))
    .bind(json!({"test": "dead-letter-resolve"}))
    .fetch_one(&pool)
    .await
    .unwrap();

    let resolve_resp = server
        .post(&format!(
            "/api/v1/compensation/dead-letters/{}/resolve",
            record_id2.0
        ))
        .await;
    resolve_resp.assert_status(StatusCode::NO_CONTENT);

    let status_after_resolve: (String,) = sqlx::query_as(
        "SELECT status::text FROM delivery_records WHERE id = $1",
    )
    .bind(record_id2.0)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        status_after_resolve.0, "delivered",
        "After resolve, status should be delivered"
    );

    // 5. GET delivery-records/{id} 验证详情可查
    let detail_resp = server
        .get(&format!(
            "/api/v1/compensation/delivery-records/{}",
            record_id2.0
        ))
        .await;
    detail_resp.assert_status(StatusCode::OK);
    let detail: serde_json::Value = detail_resp.json();
    assert_eq!(detail["id"].as_str(), Some(&*record_id2.0.to_string()));

    // 清理
    sqlx::query("DELETE FROM delivery_records WHERE route_id = $1")
        .bind(route_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn compensation_batch_retry() {
    let pool = common::test_pool().await;
    let server = common::test_server().await;

    let route_id = Uuid::new_v4();
    let mut ids = Vec::new();

    // 创建 3 条 dead 记录
    for i in 0..3 {
        let record: (Uuid,) = sqlx::query_as(
            r#"INSERT INTO delivery_records (route_id, trace_id, request_payload, status)
               VALUES ($1, $2, $3, 'dead'::delivery_status)
               RETURNING id"#,
        )
        .bind(route_id)
        .bind(format!("trace-batch-{i}-{}", Uuid::new_v4()))
        .bind(json!({"test": format!("batch-retry-{i}")}))
        .fetch_one(&pool)
        .await
        .unwrap();
        ids.push(record.0);
    }

    // 批量重试
    let resp = server
        .post("/api/v1/compensation/dead-letters/batch-retry")
        .json(&json!({"ids": ids}))
        .await;
    resp.assert_status(StatusCode::OK);
    let body: serde_json::Value = resp.json();
    assert_eq!(
        body["retried"], 3,
        "batch-retry should return retried: 3, got: {body}"
    );

    // 验证所有记录都变为 failed
    for id in &ids {
        let status: (String,) = sqlx::query_as(
            "SELECT status::text FROM delivery_records WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            status.0, "failed",
            "Record {} should be failed after batch retry",
            id
        );
    }

    // 清理
    sqlx::query("DELETE FROM delivery_records WHERE route_id = $1")
        .bind(route_id)
        .execute(&pool)
        .await
        .unwrap();
}
