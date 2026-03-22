/// 集成测试：验证 RequestLogger 和 IdempotencyGuard 与真实数据库的交互行为。
/// 每个测试使用独立的 route_id（新建 UUID 模拟），不依赖预置 fixture，
/// 测试间通过 trace_id / key 前缀隔离，避免相互干扰。
use api_anything_compensation::idempotency::IdempotencyGuard;
use api_anything_compensation::request_logger::RequestLogger;
use api_anything_common::error::AppError;
use api_anything_common::models::DeliveryGuarantee;
use api_anything_metadata::pg::PgMetadataRepo;
use sqlx::PgPool;
use uuid::Uuid;

/// 从环境变量读取 DATABASE_URL，建立连接池并运行迁移，
/// 保证每次测试运行都基于最新 schema
async fn setup_pool() -> PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = PgPool::connect(&url).await.expect("Failed to connect to DB");
    pool
}

/// 辅助函数：在数据库中插入测试所需的最小数据集（project → contract → backend_binding → route），
/// 返回 route_id，避免每个测试重复相同的 setup 代码
async fn create_test_route(repo: &PgMetadataRepo) -> Uuid {
    use api_anything_common::models::{
        DeliveryGuarantee, HttpMethod, ProtocolType, SourceType,
    };
    use api_anything_metadata::repo::MetadataRepo;

    let project = repo
        .create_project(
            &format!("test-proj-{}", Uuid::new_v4()),
            "test",
            "test-owner",
            SourceType::Wsdl,
        )
        .await
        .unwrap();

    let contract = repo
        .create_contract(
            project.id,
            "1.0",
            "<wsdl/>",
            &serde_json::json!({}),
        )
        .await
        .unwrap();

    let binding = repo
        .create_backend_binding(
            ProtocolType::Http,
            &serde_json::json!({"url": "http://example.com"}),
            5000,
        )
        .await
        .unwrap();

    let route = repo
        .create_route(
            contract.id,
            HttpMethod::Post,
            &format!("/test/{}", Uuid::new_v4()),
            &serde_json::json!({}),
            &serde_json::json!({}),
            &serde_json::json!({}),
            binding.id,
        )
        .await
        .unwrap();

    route.id
}

#[tokio::test]
async fn at_most_once_does_not_create_record() {
    let pool = setup_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let route_id = create_test_route(&repo).await;

    // AtMostOnce 是"发即忘"语义，不应写入任何投递记录
    let result = RequestLogger::log_if_needed(
        &repo,
        &DeliveryGuarantee::AtMostOnce,
        route_id,
        "trace-amo-001",
        None,
        &serde_json::json!({"action": "test"}),
    )
    .await
    .unwrap();

    assert!(
        result.is_none(),
        "AtMostOnce should not create a delivery record"
    );
}

#[tokio::test]
async fn at_least_once_creates_record() {
    let pool = setup_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let route_id = create_test_route(&repo).await;

    let result = RequestLogger::log_if_needed(
        &repo,
        &DeliveryGuarantee::AtLeastOnce,
        route_id,
        "trace-alo-001",
        None,
        &serde_json::json!({"action": "test"}),
    )
    .await
    .unwrap();

    let record = result.expect("AtLeastOnce should create a delivery record");
    assert_eq!(record.route_id, route_id);
    assert_eq!(record.trace_id, "trace-alo-001");
    assert_eq!(record.idempotency_key, None);

    // 通过 get_delivery_record 验证记录已持久化到数据库
    use api_anything_metadata::repo::MetadataRepo;
    let fetched = repo.get_delivery_record(record.id).await.unwrap();
    assert_eq!(fetched.id, record.id);
}

#[tokio::test]
async fn exactly_once_requires_idempotency_key() {
    let pool = setup_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let route_id = create_test_route(&repo).await;

    // ExactlyOnce 模式下省略 idempotency_key，应返回 BadRequest
    let result = RequestLogger::log_if_needed(
        &repo,
        &DeliveryGuarantee::ExactlyOnce,
        route_id,
        "trace-eo-001",
        None, // 故意不传 idempotency_key
        &serde_json::json!({"action": "test"}),
    )
    .await;

    assert!(
        matches!(result, Err(AppError::BadRequest(_))),
        "ExactlyOnce without key should return BadRequest, got: {:?}",
        result
    );
}

#[tokio::test]
async fn exactly_once_rejects_duplicate_key() {
    let pool = setup_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let route_id = create_test_route(&repo).await;
    // key 包含 UUID 确保测试间不冲突
    let key = format!("idem-key-{}", Uuid::new_v4());

    // 第一次请求：正常创建投递记录
    let first = RequestLogger::log_if_needed(
        &repo,
        &DeliveryGuarantee::ExactlyOnce,
        route_id,
        "trace-eo-dup-001",
        Some(&key),
        &serde_json::json!({"action": "create"}),
    )
    .await
    .unwrap();
    assert!(first.is_some(), "First ExactlyOnce request should succeed");

    // 相同 key 的第二次请求（pending 状态）应被拒绝
    let second = RequestLogger::log_if_needed(
        &repo,
        &DeliveryGuarantee::ExactlyOnce,
        route_id,
        "trace-eo-dup-002",
        Some(&key),
        &serde_json::json!({"action": "create"}),
    )
    .await;

    assert!(
        matches!(second, Err(AppError::BadRequest(_))),
        "Duplicate pending key should return BadRequest, got: {:?}",
        second
    );

    // 将幂等键标记为已投递
    IdempotencyGuard::mark_delivered(&repo, &key, "hash-abc123")
        .await
        .unwrap();

    // delivered 状态下的第三次请求应返回 AlreadyDelivered
    let third = RequestLogger::log_if_needed(
        &repo,
        &DeliveryGuarantee::ExactlyOnce,
        route_id,
        "trace-eo-dup-003",
        Some(&key),
        &serde_json::json!({"action": "create"}),
    )
    .await;

    assert!(
        matches!(third, Err(AppError::AlreadyDelivered)),
        "Delivered key should return AlreadyDelivered, got: {:?}",
        third
    );
}
