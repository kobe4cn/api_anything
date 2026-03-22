use api_anything_common::error::AppError;
use api_anything_metadata::MetadataRepo;
use serde_json::Value;
use uuid::Uuid;

pub struct ReplayLayer;

impl ReplayLayer {
    /// 从录音库中查找与当前请求匹配的历史响应；
    /// 先精确匹配，若无精确命中则按顶层 key 相似度模糊回退，
    /// 均无命中时返回 NotFound 而非静默返回空响应，使调用方能感知录音缺失
    pub async fn replay(
        repo: &impl MetadataRepo,
        session_id: Uuid,
        route_id: Uuid,
        request: &Value,
    ) -> Result<Value, AppError> {
        match repo
            .find_matching_interaction(session_id, route_id, request)
            .await?
        {
            Some(interaction) => Ok(interaction.response),
            None => Err(AppError::NotFound(
                "No matching recorded interaction found".into(),
            )),
        }
    }
}
