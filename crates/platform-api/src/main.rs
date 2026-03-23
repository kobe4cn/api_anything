use api_anything_common::config::AppConfig;
use api_anything_compensation::config::RetryConfig;
use api_anything_compensation::retry_worker::RetryWorker;
use api_anything_gateway::dispatcher::BackendDispatcher;
use api_anything_gateway::loader::RouteLoader;
use api_anything_gateway::router::DynamicRouter;
use api_anything_metadata::repo::MetadataRepo;
use api_anything_metadata::PgMetadataRepo;
use api_anything_platform_api::build_app;
use api_anything_platform_api::middleware::tracing_mw;
use api_anything_platform_api::state::AppState;
use dashmap::DashMap;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
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

    // 启动重试 worker；worker 持有 repo 和 dispatchers 的 Arc 引用，
    // 与主服务共享同一套资源，无需额外的连接池或配置
    let retry_worker = RetryWorker::new(
        repo.clone(),
        dispatchers.clone(),
        RetryConfig::default(),
    );
    tokio::spawn(async move { retry_worker.run().await });
    tracing::info!("Retry worker started");

    // 路由热加载轮询：定期检查数据库中路由数量是否变化，有变化时原子替换路由表，
    // 实现新增/删除路由后无需重启即可生效
    let poll_repo = repo.clone();
    let poll_router = router.clone();
    let poll_dispatchers = dispatchers.clone();
    tokio::spawn(async move {
        let poll_interval = Duration::from_secs(
            std::env::var("ROUTE_POLL_INTERVAL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5),
        );
        let mut last_count: usize = 0;
        let mut interval = tokio::time::interval(poll_interval);
        loop {
            interval.tick().await;
            let poll_result = poll_repo.list_active_routes_with_bindings().await;
            match poll_result {
                Ok(routes) if routes.len() != last_count => {
                    match RouteLoader::load(
                        poll_repo.as_ref(),
                        &poll_router,
                        &poll_dispatchers,
                    )
                    .await
                    {
                        Ok(loaded) => {
                            last_count = loaded;
                            tracing::info!(routes = loaded, "Routes hot-reloaded");
                        }
                        Err(e) => tracing::error!(error = %e, "Route reload failed"),
                    }
                }
                Ok(routes) => {
                    last_count = routes.len();
                }
                Err(e) => tracing::warn!(error = %e, "Route poll check failed"),
            }
        }
    });
    tracing::info!("Route polling task started");

    let app = build_app(state);
    let addr = format!("{}:{}", config.api_host, config.api_port);
    let listener = TcpListener::bind(&addr).await?;
    tracing::info!("Listening on {addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
