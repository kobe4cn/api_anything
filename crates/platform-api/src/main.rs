use api_anything_common::config::AppConfig;
use api_anything_gateway::dispatcher::BackendDispatcher;
use api_anything_gateway::loader::RouteLoader;
use api_anything_gateway::router::DynamicRouter;
use api_anything_metadata::PgMetadataRepo;
use api_anything_platform_api::build_app;
use api_anything_platform_api::middleware::tracing_mw;
use api_anything_platform_api::state::AppState;
use dashmap::DashMap;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::net::TcpListener;
use uuid::Uuid;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let config = AppConfig::from_env();
    tracing_mw::init_tracing(&config.otel_endpoint);
    tracing::info!("Starting API-Anything Platform API");

    let pool = PgPool::connect(&config.database_url).await?;
    let repo = Arc::new(PgMetadataRepo::new(pool.clone()));
    repo.run_migrations().await?;
    tracing::info!("Database migrations completed");

    // gateway 组件先于 build_app 构建，以便路由加载器在服务启动前就能填充路由表，
    // 避免启动瞬间出现短暂的"路由全部 404"窗口
    let router = Arc::new(DynamicRouter::new());
    let dispatchers: Arc<DashMap<Uuid, Arc<BackendDispatcher>>> = Arc::new(DashMap::new());
    let state = AppState {
        db: pool,
        repo: repo.clone(),
        router: router.clone(),
        dispatchers: dispatchers.clone(),
    };

    let loaded = RouteLoader::load(repo.as_ref(), &router, &dispatchers).await?;
    tracing::info!(routes = loaded, "Gateway routes loaded");

    let app = build_app(state);
    let addr = format!("{}:{}", config.api_host, config.api_port);
    let listener = TcpListener::bind(&addr).await?;
    tracing::info!("Listening on {addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
