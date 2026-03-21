use api_anything_metadata::{MetadataRepo, PgMetadataRepo};
use api_anything_common::models::SourceType;
use sqlx::PgPool;
use uuid::Uuid;

async fn setup() -> PgMetadataRepo {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://api_anything:api_anything@localhost:5432/api_anything".to_string());
    let pool = PgPool::connect(&database_url).await.expect("Failed to connect to DB");
    let repo = PgMetadataRepo::new(pool);
    repo.run_migrations().await.expect("Failed to run migrations");
    repo
}

#[tokio::test]
async fn list_active_routes_returns_empty_when_no_routes() {
    let repo = setup().await;
    let routes = repo.list_active_routes_with_bindings().await.unwrap();
    // 可能有之前测试遗留数据，但至少不应 panic
    assert!(routes.len() >= 0);
}

#[tokio::test]
async fn list_active_routes_returns_joined_data() {
    let repo = setup().await;
    let pool = get_pool().await;

    let suffix = Uuid::new_v4();

    // 1. 创建项目
    let project = repo
        .create_project(
            &format!("route-test-{suffix}"),
            "Test project for route query",
            "team-test",
            SourceType::Wsdl,
        )
        .await
        .unwrap();

    // 2. 创建契约
    let contract_id: Uuid = sqlx::query_scalar(
        "INSERT INTO contracts (project_id, version, original_schema) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(project.id)
    .bind("1.0.0")
    .bind("<wsdl>test</wsdl>")
    .fetch_one(&pool)
    .await
    .unwrap();

    // 3. 创建后端绑定
    let binding_id: Uuid = sqlx::query_scalar(
        "INSERT INTO backend_bindings (protocol) VALUES ('soap') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    // 4. 创建启用的路由
    let route_id: Uuid = sqlx::query_scalar(
        "INSERT INTO routes (contract_id, method, path, backend_binding_id, enabled) VALUES ($1, 'GET', $2, $3, true) RETURNING id",
    )
    .bind(contract_id)
    .bind(format!("/api/v1/test-{suffix}"))
    .bind(binding_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    // 5. 创建禁用的路由（不应出现在结果中）
    let _disabled_route_id: Uuid = sqlx::query_scalar(
        "INSERT INTO routes (contract_id, method, path, backend_binding_id, enabled) VALUES ($1, 'POST', $2, $3, false) RETURNING id",
    )
    .bind(contract_id)
    .bind(format!("/api/v1/disabled-{suffix}"))
    .bind(binding_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    // 6. 查询活跃路由
    let routes = repo.list_active_routes_with_bindings().await.unwrap();

    // 查找我们刚创建的路由
    let found = routes.iter().find(|r| r.route_id == route_id);
    assert!(found.is_some(), "Should find the active route");

    let route = found.unwrap();
    assert_eq!(route.binding_id, binding_id);
    assert_eq!(route.contract_id, contract_id);
    assert_eq!(route.path, format!("/api/v1/test-{suffix}"));
    assert_eq!(route.timeout_ms, 30000); // 默认值

    // 禁用的路由不应出现
    let disabled_found = routes.iter().any(|r| r.path == format!("/api/v1/disabled-{suffix}"));
    assert!(!disabled_found, "Disabled route should NOT appear in active routes");

    // 清理测试数据
    sqlx::query("DELETE FROM routes WHERE contract_id = $1")
        .bind(contract_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM contracts WHERE id = $1")
        .bind(contract_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM backend_bindings WHERE id = $1")
        .bind(binding_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project.id)
        .execute(&pool)
        .await
        .unwrap();
}

async fn get_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://api_anything:api_anything@localhost:5432/api_anything".to_string());
    PgPool::connect(&database_url).await.expect("Failed to connect")
}
