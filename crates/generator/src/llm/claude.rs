use anyhow::{anyhow, Context};
use serde_json::{json, Value};

use super::client::{extract_json_block, BoxFuture, LlmClient};

pub struct ClaudeClient {
    api_key: String,
    model: String,
    client: reqwest::Client,
}

impl ClaudeClient {
    pub fn new(api_key: String, model: Option<String>) -> Self {
        Self {
            api_key,
            // claude-sonnet-4-20250514 是截至当前发布的能力与成本平衡最优模型
            model: model.unwrap_or_else(|| "claude-sonnet-4-20250514".to_string()),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(600))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }
}

impl LlmClient for ClaudeClient {
    fn complete<'a>(
        &'a self,
        system_prompt: &'a str,
        user_prompt: &'a str,
    ) -> BoxFuture<'a, Result<String, anyhow::Error>> {
        Box::pin(async move {
            let body = json!({
                "model": self.model,
                "max_tokens": 4096,
                "system": system_prompt,
                "messages": [
                    {"role": "user", "content": user_prompt}
                ]
            });

            let resp = self
                .client
                .post("https://api.anthropic.com/v1/messages")
                // Anthropic API 使用自定义头部传递 key，而非标准 Authorization Bearer
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
                .context("Failed to send request to Claude API")?;

            let status = resp.status();
            // 在读取 body 之前检查状态码，以便提供有意义的错误信息，
            // 避免将 JSON 错误体误解析为成功响应
            if !status.is_success() {
                let err_body = resp.text().await.unwrap_or_default();
                return Err(match status.as_u16() {
                    401 => anyhow!("Claude API authentication failed (401): invalid API key"),
                    429 => anyhow!("Claude API rate limited (429): {}", err_body),
                    _ => anyhow!("Claude API error {}: {}", status, err_body),
                });
            }

            let resp_json: Value = resp
                .json()
                .await
                .context("Failed to parse Claude API response")?;

            // 从 content[0].text 中提取文本，符合 Anthropic Messages API 响应格式
            resp_json["content"][0]["text"]
                .as_str()
                .map(|s| s.to_string())
                .ok_or_else(|| anyhow!("Unexpected Claude response format: missing content[0].text"))
        })
    }

    fn complete_json<'a>(
        &'a self,
        system_prompt: &'a str,
        user_prompt: &'a str,
    ) -> BoxFuture<'a, Result<Value, anyhow::Error>> {
        Box::pin(async move {
            let text = self.complete(system_prompt, user_prompt).await?;
            let json_str = extract_json_block(&text)
                .ok_or_else(|| anyhow!("Claude response contained no extractable JSON block"))?;
            serde_json::from_str(json_str).context("Failed to parse JSON from Claude response")
        })
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}
