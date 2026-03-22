use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use api_anything_common::error::AppError;
use api_anything_metadata::repo::MetadataRepo;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::state::AppState;

#[derive(Deserialize)]
pub struct CreateWebhookRequest {
    pub url: String,
    /// 事件类型列表，空数组表示订阅全部事件
    #[serde(default)]
    pub event_types: Vec<String>,
    #[serde(default)]
    pub description: String,
}

/// 创建 Webhook 订阅；将事件类型列表序列化为 JSONB 数组存储
pub async fn create_webhook(
    State(state): State<AppState>,
    Json(req): Json<CreateWebhookRequest>,
) -> Result<impl IntoResponse, AppError> {
    let event_types: Value = serde_json::to_value(&req.event_types)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let sub = state
        .repo
        .create_webhook_subscription(&req.url, &event_types, &req.description)
        .await?;
    Ok((StatusCode::CREATED, Json(sub)))
}

/// 列出全部 Webhook 订阅，供管理界面展示和运维审计
pub async fn list_webhooks(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let subs = state.repo.list_webhook_subscriptions().await?;
    Ok(Json(subs))
}

/// 按 id 删除 Webhook 订阅
pub async fn delete_webhook(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    state.repo.delete_webhook_subscription(id).await?;
    Ok(StatusCode::NO_CONTENT)
}
