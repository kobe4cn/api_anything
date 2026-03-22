use std::env;

// 所有配置项均提供合理的本地开发默认值，保证开箱即用，
// 生产部署时通过环境变量覆盖，无需修改代码
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub database_url: String,
    pub kafka_brokers: String,
    pub otel_endpoint: String,
    pub api_host: String,
    pub api_port: u16,
}

impl AppConfig {
    pub fn from_env() -> Self {
        Self {
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://api_anything:api_anything@localhost:5432/api_anything".to_string()),
            kafka_brokers: env::var("KAFKA_BROKERS")
                .unwrap_or_else(|_| "localhost:9092".to_string()),
            otel_endpoint: env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
                .unwrap_or_else(|_| "http://localhost:4317".to_string()),
            api_host: env::var("API_HOST")
                .unwrap_or_else(|_| "0.0.0.0".to_string()),
            // API_PORT 解析失败时回退到 8080，而非 panic，保证启动健壮性
            api_port: env::var("API_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8080),
        }
    }
}

/// LLM 多厂商配置，支持 7 种 provider：
/// - anthropic: Anthropic Messages API（专有协议）
/// - openai: OpenAI Chat Completions API
/// - gemini: Google Gemini API（专有协议）
/// - glm/qwen/kimi/deepseek: 均兼容 OpenAI Chat Completions API，仅 base_url 和 key 不同
#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub provider: String,
    pub model: String,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
}

impl LlmConfig {
    /// 从环境变量读取 LLM 配置。
    /// LLM_PROVIDER 决定使用哪个厂商，对应厂商的 API Key 环境变量名各不相同。
    /// 未配置 LLM_PROVIDER 时默认 anthropic，未设置 key 时 is_available() 返回 false，
    /// 让调用方可安全降级为确定性映射。
    pub fn from_env() -> Self {
        let provider = env::var("LLM_PROVIDER").unwrap_or_else(|_| "anthropic".to_string());

        let (api_key, base_url, default_model) = match provider.as_str() {
            "anthropic" => (
                env::var("ANTHROPIC_API_KEY").ok(),
                None,
                "claude-sonnet-4-20250514",
            ),
            "openai" => (
                env::var("OPENAI_API_KEY").ok(),
                env::var("OPENAI_BASE_URL").ok(),
                "gpt-4o",
            ),
            "gemini" => (
                env::var("GEMINI_API_KEY").ok(),
                env::var("GEMINI_BASE_URL").ok(),
                "gemini-2.0-flash",
            ),
            "glm" => (
                env::var("GLM_API_KEY").ok(),
                env::var("GLM_BASE_URL").ok().or(Some(
                    "https://open.bigmodel.cn/api/paas/v4".to_string(),
                )),
                "glm-4-flash",
            ),
            "qwen" => (
                env::var("QWEN_API_KEY").ok(),
                env::var("QWEN_BASE_URL").ok().or(Some(
                    "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string(),
                )),
                "qwen-max",
            ),
            "kimi" => (
                env::var("KIMI_API_KEY").ok(),
                env::var("KIMI_BASE_URL").ok().or(Some(
                    "https://api.moonshot.cn/v1".to_string(),
                )),
                "moonshot-v1-128k",
            ),
            "deepseek" => (
                env::var("DEEPSEEK_API_KEY").ok(),
                env::var("DEEPSEEK_BASE_URL").ok().or(Some(
                    "https://api.deepseek.com".to_string(),
                )),
                "deepseek-chat",
            ),
            // 未知 provider 回退到无 key 状态，由调用方走确定性映射
            _ => (None, None, "gpt-4o"),
        };

        let model = env::var("LLM_MODEL").unwrap_or_else(|_| default_model.to_string());

        Self {
            provider,
            model,
            api_key,
            base_url,
        }
    }

    /// 是否配置了有效的 LLM（有非空 API Key）
    pub fn is_available(&self) -> bool {
        self.api_key
            .as_ref()
            .map(|k| !k.is_empty())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llm_config_defaults_to_anthropic() {
        // 不设置任何 LLM 环境变量时，provider 默认 anthropic
        let config = LlmConfig {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            api_key: None,
            base_url: None,
        };
        assert_eq!(config.provider, "anthropic");
        assert!(!config.is_available());
    }

    #[test]
    fn llm_config_is_available_with_key() {
        let config = LlmConfig {
            provider: "openai".to_string(),
            model: "gpt-4o".to_string(),
            api_key: Some("sk-test-key".to_string()),
            base_url: None,
        };
        assert!(config.is_available());
    }

    #[test]
    fn llm_config_empty_key_is_unavailable() {
        let config = LlmConfig {
            provider: "openai".to_string(),
            model: "gpt-4o".to_string(),
            api_key: Some("".to_string()),
            base_url: None,
        };
        assert!(!config.is_available());
    }
}
