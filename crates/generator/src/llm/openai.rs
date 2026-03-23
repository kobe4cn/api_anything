use anyhow::{anyhow, Context};
use serde_json::{json, Value};
use tracing;

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
            // 推理模型（如 glm-5, o1, qwen3.5）响应较慢，需要足够的超时时间
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(600))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
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
                "max_tokens": 16384
            });

            // 重试机制：超时/500/429 自动重试，最多 3 次，间隔递增
            let max_retries = 3u32;
            let mut last_error = None;

            for attempt in 0..=max_retries {
                if attempt > 0 {
                    let delay = std::time::Duration::from_secs(5 * attempt as u64);
                    tracing::warn!(attempt, max_retries, delay_secs = delay.as_secs(), "LLM API retry");
                    tokio::time::sleep(delay).await;
                }

                let result = self
                    .client
                    .post(&url)
                    .header("Authorization", format!("Bearer {}", self.api_key))
                    .header("Content-Type", "application/json")
                    .json(&body)
                    .send()
                    .await;

                let resp = match result {
                    Ok(r) => r,
                    Err(e) => {
                        // 超时或网络错误 → 重试
                        tracing::warn!(error = %e, attempt, "LLM API request failed, will retry");
                        last_error = Some(anyhow!("Request failed: {}", e));
                        continue;
                    }
                };

                let status = resp.status();
                if status.as_u16() == 401 {
                    // 认证失败不重试
                    let err_body = resp.text().await.unwrap_or_default();
                    return Err(anyhow!("OpenAI-compatible API authentication failed (401): {}", err_body));
                }

                if status.as_u16() == 429 || status.as_u16() >= 500 {
                    // 限流或服务端错误 → 重试
                    let err_body = resp.text().await.unwrap_or_default();
                    tracing::warn!(status = status.as_u16(), attempt, "LLM API server error, will retry");
                    last_error = Some(anyhow!("API error {}: {}", status, err_body));
                    continue;
                }

                if !status.is_success() {
                    let err_body = resp.text().await.unwrap_or_default();
                    return Err(anyhow!("OpenAI-compatible API error {}: {}", status, err_body));
                }

                let resp_json: Value = resp
                    .json()
                    .await
                    .context("Failed to parse OpenAI-compatible API response")?;

                return resp_json["choices"][0]["message"]["content"]
                    .as_str()
                    .map(|s| s.to_string())
                    .ok_or_else(|| {
                        anyhow!("Unexpected response format: missing choices[0].message.content")
                    });
            }

            Err(last_error.unwrap_or_else(|| anyhow!("LLM API failed after {} retries", max_retries)))
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
