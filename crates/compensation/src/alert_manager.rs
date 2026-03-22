use serde_json::json;

/// 告警通道类型：Slack 使用 text 字段的 markdown，钉钉使用 msgtype=markdown 格式
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlertType {
    Slack,
    DingTalk,
}

#[derive(Debug, Clone)]
pub struct AlertConfig {
    pub webhook_url: String,
    pub webhook_type: AlertType,
}

/// 统一告警管理器，支持 Slack 和钉钉两种 Webhook 通道；
/// 未配置时所有告警调用静默忽略，避免未集成告警的环境产生错误
pub struct AlertManager {
    config: Option<AlertConfig>,
    client: reqwest::Client,
}

impl AlertManager {
    /// 从环境变量初始化：ALERT_WEBHOOK_URL 为必需，ALERT_WEBHOOK_TYPE 缺省为 slack；
    /// URL 未设置时 config 为 None，后续 send_alert 调用直接返回 Ok
    pub fn from_env() -> Self {
        let url = std::env::var("ALERT_WEBHOOK_URL").ok();
        let alert_type = std::env::var("ALERT_WEBHOOK_TYPE")
            .unwrap_or_else(|_| "slack".to_string());
        Self {
            config: url.map(|u| AlertConfig {
                webhook_url: u,
                webhook_type: if alert_type == "dingtalk" {
                    AlertType::DingTalk
                } else {
                    AlertType::Slack
                },
            }),
            client: reqwest::Client::new(),
        }
    }

    pub fn new(config: Option<AlertConfig>) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    /// 发送告警消息；根据通道类型构造不同的 JSON body 格式
    pub async fn send_alert(
        &self,
        title: &str,
        message: &str,
        severity: &str,
    ) -> Result<(), anyhow::Error> {
        let config = match &self.config {
            Some(c) => c,
            None => return Ok(()), // 未配置告警则静默忽略
        };

        let body = match config.webhook_type {
            AlertType::Slack => json!({
                "text": format!("*[{}] {}*\n{}", severity, title, message),
            }),
            AlertType::DingTalk => json!({
                "msgtype": "markdown",
                "markdown": {
                    "title": format!("[{}] {}", severity, title),
                    "text": format!("### [{}] {}\n\n{}", severity, title, message),
                }
            }),
        };

        self.client
            .post(&config.webhook_url)
            .json(&body)
            .send()
            .await?;
        Ok(())
    }

    /// 检查死信记录并发送告警的独立函数，供后续集成任务调用；
    /// 将死信上下文格式化为告警消息，不直接修改 dead_letter.rs 的逻辑
    pub async fn alert_on_dead_letter(
        &self,
        route_id: &str,
        record_id: &str,
        error_message: &str,
        retry_count: i32,
    ) -> Result<(), anyhow::Error> {
        let title = "Dead Letter Detected";
        let message = format!(
            "Route: {}\nRecord: {}\nRetries: {}\nError: {}",
            route_id, record_id, retry_count, error_message
        );
        self.send_alert(title, &message, "CRITICAL").await
    }
}

/// 构造 Slack 格式的告警 body（公开用于测试验证）
pub fn build_slack_body(title: &str, message: &str, severity: &str) -> serde_json::Value {
    json!({
        "text": format!("*[{}] {}*\n{}", severity, title, message),
    })
}

/// 构造钉钉格式的告警 body（公开用于测试验证）
pub fn build_dingtalk_body(title: &str, message: &str, severity: &str) -> serde_json::Value {
    json!({
        "msgtype": "markdown",
        "markdown": {
            "title": format!("[{}] {}", severity, title),
            "text": format!("### [{}] {}\n\n{}", severity, title, message),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slack_body_format() {
        let body = build_slack_body("Test Alert", "Something went wrong", "WARN");
        let text = body["text"].as_str().unwrap();
        assert!(text.contains("*[WARN] Test Alert*"));
        assert!(text.contains("Something went wrong"));
    }

    #[test]
    fn dingtalk_body_format() {
        let body = build_dingtalk_body("Test Alert", "Something went wrong", "CRITICAL");
        assert_eq!(body["msgtype"], "markdown");
        let md = &body["markdown"];
        assert!(md["title"].as_str().unwrap().contains("[CRITICAL] Test Alert"));
        assert!(md["text"].as_str().unwrap().contains("### [CRITICAL] Test Alert"));
        assert!(md["text"].as_str().unwrap().contains("Something went wrong"));
    }

    #[test]
    fn alert_manager_without_config_is_noop() {
        let mgr = AlertManager::new(None);
        assert!(mgr.config.is_none());
    }

    #[test]
    fn alert_manager_with_slack_config() {
        let mgr = AlertManager::new(Some(AlertConfig {
            webhook_url: "https://hooks.slack.com/test".to_string(),
            webhook_type: AlertType::Slack,
        }));
        assert_eq!(mgr.config.as_ref().unwrap().webhook_type, AlertType::Slack);
    }

    #[test]
    fn alert_manager_with_dingtalk_config() {
        let mgr = AlertManager::new(Some(AlertConfig {
            webhook_url: "https://oapi.dingtalk.com/robot/send?token=xxx".to_string(),
            webhook_type: AlertType::DingTalk,
        }));
        assert_eq!(mgr.config.as_ref().unwrap().webhook_type, AlertType::DingTalk);
    }
}
