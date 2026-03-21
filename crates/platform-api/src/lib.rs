pub mod middleware;
pub mod routes;
pub mod state;

use axum::routing::{any, delete, get, post};
use axum::Router;
use api_anything_gateway::dispatcher::BackendDispatcher;
use api_anything_gateway::router::DynamicRouter;
use api_anything_metadata::PgMetadataRepo;
use dashmap::DashMap;
use sqlx::PgPool;
use state::AppState;
use std::sync::Arc;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

// build_app 暴露为公共函数，使集成测试可以在不启动 TcpListener 的情况下
// 直接构造 Router，避免端口占用和并发冲突；
// gateway 组件（router/dispatchers）在此初始化为空表，
// 运行时由路由加载任务动态填充
pub fn build_app(pool: PgPool) -> Router {
    let repo = Arc::new(PgMetadataRepo::new(pool.clone()));
    let router = Arc::new(DynamicRouter::new());
    let dispatchers: Arc<DashMap<Uuid, Arc<BackendDispatcher>>> = Arc::new(DashMap::new());
    let state = AppState { db: pool, repo, router, dispatchers };

    Router::new()
        .route("/health", get(routes::health::health))
        .route("/health/ready", get(routes::health::ready))
        .route(
            "/api/v1/projects",
            post(routes::projects::create_project).get(routes::projects::list_projects),
        )
        .route(
            "/api/v1/projects/{id}",
            get(routes::projects::get_project).delete(routes::projects::delete_project),
        )
        // 通配路由捕获所有 /gw/ 前缀请求，交由动态路由器分发
        .route("/gw/{*rest}", any(routes::gateway::gateway_handler))
        .fallback(middleware::error_handler::fallback)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
