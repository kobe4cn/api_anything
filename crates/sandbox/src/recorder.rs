use api_anything_gateway::types::GatewayResponse;
use api_anything_metadata::MetadataRepo;
use serde_json::Value;
use uuid::Uuid;

pub struct Recorder;

impl Recorder {
    /// 将一次完整的网关交互持久化到数据库；
    /// response 先序列化为 Value，使录音格式与 replay 阶段读取格式保持一致，
    /// 避免因字节层面差异导致回放时反序列化失败
    pub async fn record(
        repo: &impl MetadataRepo,
        session_id: Uuid,
        route_id: Uuid,
        request_body: &Value,
        response: &GatewayResponse,
        duration_ms: i32,
    ) -> Result<(), anyhow::Error> {
        let response_value = serde_json::to_value(response)?;
        repo.record_interaction(session_id, route_id, request_body, &response_value, duration_ms)
            .await?;
        Ok(())
    }
}
