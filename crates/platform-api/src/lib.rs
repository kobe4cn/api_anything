pub mod middleware;
pub mod routes;
pub mod state;

use axum::{routing::{delete, get, post}, Router};
use api_anything_metadata::PgMetadataRepo;
use sqlx::PgPool;
use state::AppState;
use std::sync::Arc;
use tower_http::trace::TraceLayer;

// build_app 暴露为公共函数，使集成测试可以在不启动 TcpListener 的情况下
// 直接构造 Router，避免端口占用和并发冲突
pub fn build_app(pool: PgPool) -> Router {
    let repo = Arc::new(PgMetadataRepo::new(pool.clone()));
    let state = AppState { db: pool, repo };

    Router::new()
        .route("/health", get(routes::health::health))
        .route("/health/ready", get(routes::health::ready))
        .route("/api/v1/projects", post(routes::projects::create_project).get(routes::projects::list_projects))
        .route("/api/v1/projects/{id}", get(routes::projects::get_project).delete(routes::projects::delete_project))
        .fallback(middleware::error_handler::fallback)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
