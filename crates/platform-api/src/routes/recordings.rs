use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use api_anything_common::error::AppError;
use api_anything_metadata::MetadataRepo;
use serde_json::json;
use uuid::Uuid;
use crate::state::AppState;

/// GET /api/v1/sandbox-sessions/{id}/recordings
/// 列出指定沙箱会话的所有录制数据，按录制时间倒序返回
pub async fn list_recordings(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    // 先验证会话存在，避免对不存在的会话返回空数组导致调用方误以为会话没有录制
    let _ = state.repo.get_sandbox_session(session_id).await?;
    let recordings = state.repo.list_recorded_interactions(session_id).await?;
    Ok(Json(recordings))
}

/// DELETE /api/v1/sandbox-sessions/{id}/recordings
/// 清空指定沙箱会话的所有录制数据，返回被删除的记录数
pub async fn clear_recordings(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let _ = state.repo.get_sandbox_session(session_id).await?;
    let deleted = state.repo.delete_recorded_interactions(session_id).await?;
    Ok((StatusCode::OK, Json(json!({ "deleted": deleted }))))
}
