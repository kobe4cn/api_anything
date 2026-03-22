use api_anything_common::error::AppError;
use api_anything_metadata::repo::MetadataRepo;

pub struct IdempotencyGuard;

impl IdempotencyGuard {
    /// 投递成功后调用，将幂等键状态从 pending 推进到 delivered，
    /// 并记录响应摘要；后续相同 key 的请求在 check_idempotency 阶段
    /// 即可识别为重复，直接返回 200 无需重新调用后端
    pub async fn mark_delivered(
        repo: &impl MetadataRepo,
        key: &str,
        response_hash: &str,
    ) -> Result<(), AppError> {
        repo.mark_idempotency_delivered(key, response_hash).await
    }
}
