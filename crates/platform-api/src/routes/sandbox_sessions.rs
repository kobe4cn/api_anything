use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use api_anything_common::error::AppError;
use api_anything_common::models::SandboxMode;
use api_anything_metadata::MetadataRepo;
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct CreateSandboxSessionRequest {
    pub tenant_id: String,
    pub mode: SandboxMode,
    pub config: serde_json::Value,
    /// 会话有效期（小时），由调用方指定，使平台层能针对不同租户执行不同的过期策略
    pub expires_in_hours: i64,
}

pub async fn create_sandbox_session(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Json(req): Json<CreateSandboxSessionRequest>,
) -> Result<impl IntoResponse, AppError> {
    // expires_at 在应用层计算，避免数据库时钟与应用时钟不一致导致的过期时间偏差
    let expires_at = Utc::now() + chrono::Duration::hours(req.expires_in_hours);
    let session = state
        .repo
        .create_sandbox_session(project_id, &req.tenant_id, req.mode, &req.config, expires_at)
        .await?;
    Ok((StatusCode::CREATED, Json(session)))
}

pub async fn list_sandbox_sessions(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let sessions = state.repo.list_sandbox_sessions(project_id).await?;
    Ok(Json(sessions))
}

pub async fn delete_sandbox_session(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    state.repo.delete_sandbox_session(id).await?;
    Ok(StatusCode::NO_CONTENT)
}
