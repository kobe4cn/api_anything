use api_anything_gateway::dispatcher::BackendDispatcher;
use api_anything_gateway::router::DynamicRouter;
use api_anything_metadata::PgMetadataRepo;
use api_anything_platform_api::build_app;
use api_anything_platform_api::state::AppState;
use axum_test::TestServer;
use dashmap::DashMap;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

/// 启动测试服务器时手动构造 AppState，与 main.rs 保持一致；
/// gateway 组件初始化为空表，测试可按需通过 RouteLoader 或直接插入填充
pub async fn test_server() -> TestServer {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://api_anything:api_anything@localhost:5432/api_anything".to_string());
    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test DB");
    let repo = Arc::new(PgMetadataRepo::new(pool.clone()));
    repo.run_migrations().await.expect("Failed to run migrations");

    let router = Arc::new(DynamicRouter::new());
    let dispatchers: Arc<DashMap<Uuid, Arc<BackendDispatcher>>> = Arc::new(DashMap::new());
    let state = AppState { db: pool, repo, router, dispatchers };

    let app = build_app(state);
    TestServer::new(app).unwrap()
}

/// 返回数据库连接池，供需要直接操作数据库的测试使用
pub async fn test_pool() -> PgPool {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://api_anything:api_anything@localhost:5432/api_anything".to_string());
    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test DB");
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.expect("Failed to run migrations");
    pool
}
