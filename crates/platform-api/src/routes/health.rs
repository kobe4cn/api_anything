use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use crate::state::AppState;

// /health 只检测进程存活，不依赖外部依赖，
// 让 liveness probe 和 readiness probe 语义分离
pub async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok" }))
}

// /health/ready 主动探测数据库连通性，
// 确保 readiness probe 只在服务真正可以处理流量时返回 200
pub async fn ready(State(state): State<AppState>) -> impl IntoResponse {
    match sqlx::query("SELECT 1").execute(&state.db).await {
        Ok(_) => (
            StatusCode::OK,
            Json(json!({ "status": "ready", "db": "connected" })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "Database readiness check failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "status": "not_ready", "db": "disconnected" })),
            )
                .into_response()
        }
    }
}
