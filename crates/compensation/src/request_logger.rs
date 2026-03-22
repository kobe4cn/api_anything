use api_anything_common::error::AppError;
use api_anything_common::models::{DeliveryGuarantee, DeliveryRecord};
use api_anything_metadata::repo::MetadataRepo;
use serde_json::Value;
use uuid::Uuid;

pub struct RequestLogger;

impl RequestLogger {
    /// 根据投递语义决定是否记录请求；
    /// AtMostOnce 是"发即忘"，不需要持久化记录，
    /// AtLeastOnce 写入投递记录以便失败后重试，
    /// ExactlyOnce 在写入投递记录前先检查幂等键，防止重复处理
    pub async fn log_if_needed(
        repo: &impl MetadataRepo,
        delivery_guarantee: &DeliveryGuarantee,
        route_id: Uuid,
        trace_id: &str,
        idempotency_key: Option<&str>,
        request_payload: &Value,
    ) -> Result<Option<DeliveryRecord>, AppError> {
        match delivery_guarantee {
            DeliveryGuarantee::AtMostOnce => Ok(None),
            DeliveryGuarantee::AtLeastOnce => {
                // 幂等键在 AtLeastOnce 模式下不参与去重，
                // 客户端通过 trace_id 关联重试记录
                let record = repo
                    .create_delivery_record(route_id, trace_id, None, request_payload)
                    .await?;
                Ok(Some(record))
            }
            DeliveryGuarantee::ExactlyOnce => {
                // ExactlyOnce 语义强制要求 Idempotency-Key，
                // 缺少该头部时提前拒绝而非静默降级为 AtLeastOnce
                let key = idempotency_key.ok_or_else(|| {
                    AppError::BadRequest(
                        "Idempotency-Key header required for exactly-once delivery".into(),
                    )
                })?;

                // 检查幂等键是否已存在；delivered 表示已成功投递，
                // pending 表示正在处理中（防止并发重入）
                if let Some(existing) = repo.check_idempotency(key).await? {
                    if existing.status == "delivered" {
                        return Err(AppError::AlreadyDelivered);
                    }
                    // pending 状态：另一个请求正在处理同一个 key，
                    // 返回 409 而非 200，调用方需稍后重试或查询状态
                    return Err(AppError::BadRequest(
                        "Request is already being processed".into(),
                    ));
                }

                // 先写幂等键（pending），再写投递记录；
                // 若写入投递记录失败，幂等键仍为 pending，
                // 客户端可通过超时后重试触发清理
                repo.create_idempotency_record(key, route_id).await?;
                let record = repo
                    .create_delivery_record(route_id, trace_id, Some(key), request_payload)
                    .await?;
                Ok(Some(record))
            }
        }
    }
}
