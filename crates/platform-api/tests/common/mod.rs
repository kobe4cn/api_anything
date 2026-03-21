use api_anything_metadata::PgMetadataRepo;
use api_anything_platform_api::build_app;
use axum_test::TestServer;
use sqlx::PgPool;

pub async fn test_server() -> TestServer {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://api_anything:api_anything@localhost:5432/api_anything".to_string());
    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test DB");
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.expect("Failed to run migrations");
    let app = build_app(pool);
    TestServer::new(app).unwrap()
}
