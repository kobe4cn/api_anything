use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use api_anything_common::error::AppError;
use api_anything_compensation::dead_letter::DeadLetterProcessor;
use api_anything_metadata::repo::MetadataRepo;
use serde::Deserialize;
use uuid::Uuid;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct DeadLetterQuery {
    pub route_id: Option<Uuid>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// 列出死信队列；route_id 为 None 时返回全局视图，方便运维快速评估整体积压量
pub async fn list_dead_letters(
    State(state): State<AppState>,
    Query(query): Query<DeadLetterQuery>,
) -> Result<impl IntoResponse, AppError> {
    let records = state.repo.list_dead_letters(
        query.route_id,
        query.limit.unwrap_or(50),
        query.offset.unwrap_or(0),
    ).await?;
    Ok(Json(records))
}

/// 按 id 查询单条投递记录，供运维查看原始请求体和错误详情以决定是否人工干预
pub async fn get_delivery_record(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let record = state.repo.get_delivery_record(id).await?;
    Ok(Json(record))
}

/// 将单条死信重置回 Failed + next_retry_at = now()，触发重试 worker 立即重新处理
pub async fn retry_dead_letter(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    DeadLetterProcessor::retry_dead_letter(state.repo.as_ref(), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct BatchRetryRequest {
    pub ids: Vec<Uuid>,
}

/// 批量重试死信；单条失败不中止整批，响应体返回成功重置数量
pub async fn batch_retry(
    State(state): State<AppState>,
    Json(req): Json<BatchRetryRequest>,
) -> Result<impl IntoResponse, AppError> {
    let count = DeadLetterProcessor::retry_batch(state.repo.as_ref(), &req.ids).await?;
    Ok(Json(serde_json::json!({"retried": count})))
}

/// 将死信标记为已人工解决，不再重试；适用于上游已通过其他渠道完成投递的场景
pub async fn resolve_dead_letter(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    DeadLetterProcessor::mark_resolved(state.repo.as_ref(), id).await?;
    Ok(StatusCode::NO_CONTENT)
}
