pub mod middleware;
pub mod routes;
pub mod state;

use axum::routing::{any, delete, get, post};
use axum::Router;
use state::AppState;
use tower_http::services::{ServeDir, ServeFile};
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
        // 沙箱会话路由：创建和列表挂在 project 下，删除单独挂顶层以便通过 session id 直接操作
        .route(
            "/api/v1/projects/{project_id}/sandbox-sessions",
            post(routes::sandbox_sessions::create_sandbox_session)
                .get(routes::sandbox_sessions::list_sandbox_sessions),
        )
        .route(
            "/api/v1/sandbox-sessions/{id}",
            delete(routes::sandbox_sessions::delete_sandbox_session),
        )
        // 补偿系统管理 API：死信队列查询、重试、批量重试、人工解决
        .route(
            "/api/v1/compensation/dead-letters",
            get(routes::compensation::list_dead_letters),
        )
        .route(
            "/api/v1/compensation/dead-letters/batch-retry",
            post(routes::compensation::batch_retry),
        )
        .route(
            "/api/v1/compensation/dead-letters/{id}/retry",
            post(routes::compensation::retry_dead_letter),
        )
        .route(
            "/api/v1/compensation/dead-letters/{id}/resolve",
            post(routes::compensation::resolve_dead_letter),
        )
        .route(
            "/api/v1/compensation/delivery-records/{id}",
            get(routes::compensation::get_delivery_record),
        )
        // 文档类端点：OpenAPI JSON 规范、Swagger UI、Agent 提示词
        .route("/api/v1/docs", get(routes::docs::swagger_ui))
        .route("/api/v1/docs/openapi.json", get(routes::docs::openapi_json))
        .route("/api/v1/docs/agent-prompt", get(routes::docs::agent_prompt))
        // 通配路由捕获所有 /gw/ 前缀请求，交由动态路由器分发
        .route("/gw/{*rest}", any(routes::gateway::gateway_handler))
        // 沙箱通配路由：与网关共享路由表，但根据 X-Sandbox-Mode 头走 mock/replay/proxy 分支
        .route("/sandbox/{*rest}", any(routes::sandbox::sandbox_handler))
        // 静态文件兜底：优先尝试从 web/dist 提供静态资源（JS/CSS/图片等），
        // 未匹配的路径回退到 index.html 以支持 SPA 客户端路由（React Router）
        .fallback_service(
            ServeDir::new("web/dist")
                .not_found_service(ServeFile::new("web/dist/index.html")),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
