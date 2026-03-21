use api_anything_common::config::AppConfig;
use api_anything_metadata::PgMetadataRepo;
use api_anything_platform_api::build_app;
use api_anything_platform_api::middleware::tracing_mw;
use sqlx::PgPool;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let config = AppConfig::from_env();
    tracing_mw::init_tracing(&config.otel_endpoint);
    tracing::info!("Starting API-Anything Platform API");

    let pool = PgPool::connect(&config.database_url).await?;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await?;
    tracing::info!("Database migrations completed");

    let app = build_app(pool);
    let addr = format!("{}:{}", config.api_host, config.api_port);
    let listener = TcpListener::bind(&addr).await?;
    tracing::info!("Listening on {addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
