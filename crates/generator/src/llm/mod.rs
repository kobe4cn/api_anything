pub mod client;
pub mod claude;
pub mod gemini;
pub mod openai;

use api_anything_common::config::LlmConfig;

/// 根据 LlmConfig 创建对应的 LLM 客户端实例。
/// 无有效 API Key 时返回 None，调用方应降级为确定性映射。
///
/// 只有 3 种客户端实现覆盖 7 个 provider：
/// - ClaudeClient: Anthropic 专有 Messages API
/// - OpenAiCompatibleClient: OpenAI / GLM / Qwen / Kimi / DeepSeek（协议相同，仅 base_url 不同）
/// - GeminiClient: Google Gemini 专有 API
pub fn create_llm_client(config: &LlmConfig) -> Option<Box<dyn client::LlmClient>> {
    if !config.is_available() {
        tracing::info!(
            provider = %config.provider,
            "No LLM API key configured, using deterministic mapping"
        );
        return None;
    }

    let key = config.api_key.clone().unwrap();

    match config.provider.as_str() {
        "anthropic" => Some(Box::new(claude::ClaudeClient::new(
            key,
            Some(config.model.clone()),
        ))),
        "gemini" => Some(Box::new(gemini::GeminiClient::new(
            key,
            Some(config.model.clone()),
            config.base_url.clone(),
        ))),
        // OpenAI 兼容系列共享同一客户端实现
        "openai" | "glm" | "qwen" | "kimi" | "deepseek" => {
            Some(Box::new(openai::OpenAiCompatibleClient::new(
                key,
                config.model.clone(),
                config.base_url.clone(),
            )))
        }
        unknown => {
            tracing::warn!(
                provider = unknown,
                "Unknown LLM provider, using deterministic mapping"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_none_when_no_api_key() {
        let config = LlmConfig {
            provider: "openai".to_string(),
            model: "gpt-4o".to_string(),
            api_key: None,
            base_url: None,
        };
        assert!(create_llm_client(&config).is_none());
    }

    #[test]
    fn returns_none_for_empty_api_key() {
        let config = LlmConfig {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            api_key: Some("".to_string()),
            base_url: None,
        };
        assert!(create_llm_client(&config).is_none());
    }

    #[test]
    fn returns_none_for_unknown_provider() {
        let config = LlmConfig {
            provider: "unknown-provider".to_string(),
            model: "some-model".to_string(),
            api_key: Some("some-key".to_string()),
            base_url: None,
        };
        assert!(create_llm_client(&config).is_none());
    }

    #[test]
    fn creates_claude_client() {
        let config = LlmConfig {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            api_key: Some("sk-ant-test".to_string()),
            base_url: None,
        };
        let client = create_llm_client(&config);
        assert!(client.is_some());
        assert_eq!(client.unwrap().model_name(), "claude-sonnet-4-20250514");
    }

    #[test]
    fn creates_openai_client() {
        let config = LlmConfig {
            provider: "openai".to_string(),
            model: "gpt-4o".to_string(),
            api_key: Some("sk-test".to_string()),
            base_url: None,
        };
        let client = create_llm_client(&config);
        assert!(client.is_some());
        assert_eq!(client.unwrap().model_name(), "gpt-4o");
    }

    #[test]
    fn creates_gemini_client() {
        let config = LlmConfig {
            provider: "gemini".to_string(),
            model: "gemini-2.0-flash".to_string(),
            api_key: Some("AIza-test".to_string()),
            base_url: None,
        };
        let client = create_llm_client(&config);
        assert!(client.is_some());
        assert_eq!(client.unwrap().model_name(), "gemini-2.0-flash");
    }

    /// GLM/Qwen/Kimi/DeepSeek 都走 OpenAiCompatibleClient，
    /// 验证它们能正确创建且 model_name 返回对应值
    #[test]
    fn creates_openai_compatible_clients_for_chinese_providers() {
        let providers = vec![
            ("glm", "glm-4-flash"),
            ("qwen", "qwen-max"),
            ("kimi", "moonshot-v1-128k"),
            ("deepseek", "deepseek-chat"),
        ];

        for (provider, model) in providers {
            let config = LlmConfig {
                provider: provider.to_string(),
                model: model.to_string(),
                api_key: Some("test-key".to_string()),
                base_url: Some("https://example.com/v1".to_string()),
            };
            let client = create_llm_client(&config);
            assert!(client.is_some(), "Provider {} should create a client", provider);
            assert_eq!(client.unwrap().model_name(), model);
        }
    }
}
