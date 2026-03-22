use anyhow::{anyhow, Context};
use serde_json::{json, Value};

use super::client::{extract_json_block, BoxFuture, LlmClient};

/// 通用 OpenAI Chat Completions 兼容客户端。
/// 除 OpenAI 外，GLM (智谱)、Qwen (通义千问)、Kimi (月之暗面)、DeepSeek
/// 均实现了相同的 Chat Completions 协议，只需替换 base_url 和 api_key。
pub struct OpenAiCompatibleClient {
    api_key: String,
    model: String,
    base_url: String,
    client: reqwest::Client,
}

impl OpenAiCompatibleClient {
    pub fn new(api_key: String, model: String, base_url: Option<String>) -> Self {
        Self {
            api_key,
            model,
            base_url: base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
            client: reqwest::Client::new(),
        }
    }
}

impl LlmClient for OpenAiCompatibleClient {
    fn complete<'a>(
        &'a self,
        system_prompt: &'a str,
        user_prompt: &'a str,
    ) -> BoxFuture<'a, Result<String, anyhow::Error>> {
        Box::pin(async move {
            let url = format!(
                "{}/chat/completions",
                self.base_url.trim_end_matches('/')
            );
            let body = json!({
                "model": self.model,
                "messages": [
                    {"role": "system", "content": system_prompt},
                    {"role": "user", "content": user_prompt}
                ],
                "temperature": 0.1,
                "max_tokens": 4096
            });

            let resp = self
                .client
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
                .context("Failed to send request to OpenAI-compatible API")?;

            let status = resp.status();
            if !status.is_success() {
                let err_body = resp.text().await.unwrap_or_default();
                return Err(match status.as_u16() {
                    401 => anyhow!(
                        "OpenAI-compatible API authentication failed (401): invalid API key"
                    ),
                    429 => anyhow!("OpenAI-compatible API rate limited (429): {}", err_body),
                    _ => anyhow!("OpenAI-compatible API error {}: {}", status, err_body),
                });
            }

            let resp_json: Value = resp
                .json()
                .await
                .context("Failed to parse OpenAI-compatible API response")?;

            resp_json["choices"][0]["message"]["content"]
                .as_str()
                .map(|s| s.to_string())
                .ok_or_else(|| {
                    anyhow!(
                        "Unexpected response format: missing choices[0].message.content"
                    )
                })
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
                .ok_or_else(|| anyhow!("Response contained no extractable JSON block"))?;
            serde_json::from_str(json_str).context("Failed to parse JSON from response")
        })
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}
