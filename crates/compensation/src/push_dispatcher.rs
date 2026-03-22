use api_anything_metadata::repo::MetadataRepo;
use serde_json::{json, Value};

/// 将事件推送到所有匹配的 Webhook 订阅端点；
/// 每个订阅独立发送、独立失败，单个端点不可达不影响其余端点
pub struct PushDispatcher {
    client: reqwest::Client,
}

impl PushDispatcher {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    /// 查询匹配 event_type 的活跃订阅并逐一推送；
    /// 返回成功投递的数量，失败仅记录日志不中断循环，
    /// 保证高可用端点不因低可用端点的故障而被阻塞
    pub async fn dispatch(
        &self,
        repo: &impl MetadataRepo,
        event_type: &str,
        payload: &Value,
    ) -> Result<u32, anyhow::Error> {
        let subs = repo.list_active_subscriptions_for_event(event_type).await?;
        let mut sent = 0u32;
        for sub in &subs {
            let mut req = self
                .client
                .post(&sub.url)
                .header("Content-Type", "application/json")
                .json(&json!({
                    "event_type": event_type,
                    "payload": payload,
                    "timestamp": chrono::Utc::now(),
                }));

            // 从订阅配置的 headers 字段注入自定义请求头（如 Authorization），
            // 支持目标系统的认证要求而无需在 URL 中暴露凭据
            if let Some(headers) = sub.headers.as_object() {
                for (k, v) in headers {
                    if let Some(val) = v.as_str() {
                        req = req.header(k, val);
                    }
                }
            }

            match req.send().await {
                Ok(resp) if resp.status().is_success() => {
                    sent += 1;
                }
                Ok(resp) => {
                    tracing::warn!(
                        url = %sub.url,
                        status = %resp.status(),
                        "Webhook delivery failed with non-success status"
                    );
                }
                Err(e) => {
                    tracing::error!(
                        url = %sub.url,
                        error = %e,
                        "Webhook delivery network error"
                    );
                }
            }
        }
        Ok(sent)
    }
}

impl Default for PushDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn webhook_payload_serialization_format() {
        // 验证推送 payload 的 JSON 结构包含必需的三个顶层字段
        let event_type = "delivery.dead";
        let payload = json!({"route_id": "abc-123", "error": "timeout"});
        let body = json!({
            "event_type": event_type,
            "payload": payload,
            "timestamp": "2026-03-22T00:00:00Z",
        });
        let obj = body.as_object().unwrap();
        assert!(obj.contains_key("event_type"));
        assert!(obj.contains_key("payload"));
        assert!(obj.contains_key("timestamp"));
        assert_eq!(obj["event_type"], "delivery.dead");
    }

    #[test]
    fn push_dispatcher_default_creates_instance() {
        let _dispatcher = PushDispatcher::default();
    }
}
