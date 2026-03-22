use api_anything_common::error::AppError;
use api_anything_common::models::*;
use api_anything_metadata::repo::MetadataRepo;
use uuid::Uuid;

pub struct DeadLetterProcessor;

impl DeadLetterProcessor {
    /// 将死信记录重置为 Failed + next_retry_at = now()，
    /// 使重试 worker 在下次轮询时立即捡起该记录重新尝试投递
    pub async fn retry_dead_letter(repo: &impl MetadataRepo, record_id: Uuid) -> Result<(), AppError> {
        let record = repo.get_delivery_record(record_id).await?;
        // 只允许对 dead 状态的记录执行手动重试，
        // 防止对仍在重试队列中的 failed 记录重复操作造成计数混乱
        if record.status != DeliveryStatus::Dead {
            return Err(AppError::BadRequest("Record is not in dead letter state".into()));
        }
        repo.update_delivery_status(
            record_id,
            DeliveryStatus::Failed,
            None,
            Some(chrono::Utc::now()),
        ).await?;
        Ok(())
    }

    /// 批量重试死信；单条失败不中止整批，保证其他记录仍能被调度，
    /// 返回成功重置的数量供调用方展示操作结果
    pub async fn retry_batch(repo: &impl MetadataRepo, ids: &[Uuid]) -> Result<u32, AppError> {
        let mut count = 0;
        for id in ids {
            if Self::retry_dead_letter(repo, *id).await.is_ok() {
                count += 1;
            }
        }
        Ok(count)
    }

    /// 将记录标记为已人工处理（delivered + error_message = "Manually resolved"），
    /// 适用于上游系统已通过其他渠道完成投递、无需再重试的场景。
    /// 先验证记录存在，不存在时返回 404 而非静默 UPDATE 0 行
    pub async fn mark_resolved(repo: &impl MetadataRepo, record_id: Uuid) -> Result<(), AppError> {
        let _record = repo.get_delivery_record(record_id).await?;
        repo.update_delivery_status(
            record_id,
            DeliveryStatus::Delivered,
            Some("Manually resolved"),
            None,
        ).await
    }
}
