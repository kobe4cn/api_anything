pub mod middleware;
pub mod routes;
pub mod state;

use axum::routing::{any, delete, get, post};
use axum::Router;
use state::AppState;
use tower_http::trace::TraceLayer;

// build_app 接受已初始化的 AppState，使 main.rs 可在启动前先执行路由加载，
// 测试代码也可以传入预填充的 state 来验证特定场景
pub fn build_app(state: AppState) -> Router {
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
