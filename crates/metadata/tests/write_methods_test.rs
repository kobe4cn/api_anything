use api_anything_metadata::{MetadataRepo, PgMetadataRepo};
use api_anything_common::models::*;
use sqlx::PgPool;
use uuid::Uuid;

async fn setup() -> (PgMetadataRepo, PgPool) {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://api_anything:api_anything@localhost:5432/api_anything".to_string());
    let pool = PgPool::connect(&database_url).await.expect("Failed to connect to DB");
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.expect("Failed to run migrations");
    (repo, pool)
}

#[tokio::test]
async fn create_contract_returns_valid_record() {
    let (repo, pool) = setup().await;
    let suffix = Uuid::new_v4();

    let project = repo
        .create_project(&format!("contract-test-{suffix}"), "test", "team", SourceType::Wsdl)
        .await
        .unwrap();

    let contract = repo
        .create_contract(
            project.id,
            "1.0.0",
            "<wsdl>test content</wsdl>",
            &serde_json::json!({"operations": []}),
        )
        .await
        .unwrap();

    assert_eq!(contract.project_id, project.id);
    assert_eq!(contract.version, "1.0.0");
    assert_eq!(contract.original_schema, "<wsdl>test content</wsdl>");
    assert_eq!(contract.status, ContractStatus::Draft);

    // 清理
    sqlx::query("DELETE FROM projects WHERE id = $1").bind(project.id).execute(&pool).await.unwrap();
}

#[tokio::test]
async fn create_backend_binding_with_defaults() {
    let (repo, pool) = setup().await;

    let binding = repo
        .create_backend_binding(
            ProtocolType::Soap,
            &serde_json::json!({"url": "http://example.com/soap"}),
            30000,
        )
        .await
        .unwrap();

    assert_eq!(binding.protocol, ProtocolType::Soap);
    assert_eq!(binding.timeout_ms, 30000);
    // 连接池/熔断/限流应使用数据库默认值
    assert!(binding.connection_pool_config.is_object());
    assert!(binding.circuit_breaker_config.is_object());

    // 清理
    sqlx::query("DELETE FROM backend_bindings WHERE id = $1").bind(binding.id).execute(&pool).await.unwrap();
}

#[tokio::test]
async fn create_route_linked_to_contract_and_binding() {
    let (repo, pool) = setup().await;
    let suffix = Uuid::new_v4();

    let project = repo
        .create_project(&format!("route-create-test-{suffix}"), "test", "team", SourceType::Wsdl)
        .await
        .unwrap();

    let contract = repo
        .create_contract(project.id, "1.0.0", "<wsdl/>", &serde_json::json!({}))
        .await
        .unwrap();

    let binding = repo
        .create_backend_binding(ProtocolType::Soap, &serde_json::json!({}), 30000)
        .await
        .unwrap();

    let route = repo
        .create_route(
            contract.id,
            HttpMethod::Post,
            &format!("/api/v1/test-{suffix}/add"),
            &serde_json::json!({"type": "object", "properties": {"a": {"type": "integer"}}}),
            &serde_json::json!({"type": "object", "properties": {"result": {"type": "integer"}}}),
            &serde_json::json!({"soap_action": "Add"}),
            binding.id,
        )
        .await
        .unwrap();

    assert_eq!(route.contract_id, contract.id);
    assert_eq!(route.method, HttpMethod::Post);
    assert!(route.path.contains("add"));
    assert_eq!(route.backend_binding_id, binding.id);
    assert!(route.enabled); // 默认启用
    assert_eq!(route.delivery_guarantee, DeliveryGuarantee::AtMostOnce); // 默认值

    // 清理
    sqlx::query("DELETE FROM projects WHERE id = $1").bind(project.id).execute(&pool).await.unwrap();
    sqlx::query("DELETE FROM backend_bindings WHERE id = $1").bind(binding.id).execute(&pool).await.unwrap();
}

#[tokio::test]
async fn create_contract_enforces_unique_version_per_project() {
    let (repo, pool) = setup().await;
    let suffix = Uuid::new_v4();

    let project = repo
        .create_project(&format!("unique-version-test-{suffix}"), "test", "team", SourceType::Wsdl)
        .await
        .unwrap();

    // 第一次创建成功
    repo.create_contract(project.id, "1.0.0", "v1", &serde_json::json!({}))
        .await
        .unwrap();

    // 同版本号应失败（UNIQUE 约束）
    let result = repo.create_contract(project.id, "1.0.0", "v1-dup", &serde_json::json!({})).await;
    assert!(result.is_err());

    // 不同版本号应成功
    repo.create_contract(project.id, "2.0.0", "v2", &serde_json::json!({}))
        .await
        .unwrap();

    // 清理
    sqlx::query("DELETE FROM projects WHERE id = $1").bind(project.id).execute(&pool).await.unwrap();
}
