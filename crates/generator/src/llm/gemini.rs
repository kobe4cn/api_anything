use anyhow::{anyhow, Context};
use serde_json::{json, Value};

use super::client::{extract_json_block, BoxFuture, LlmClient};

/// Google Gemini API 客户端。
/// Gemini 使用独立的 REST API 格式（非 OpenAI 兼容），
/// 认证通过 URL query parameter 传递 API key，请求体结构也不同。
pub struct GeminiClient {
    api_key: String,
    model: String,
    base_url: String,
    client: reqwest::Client,
}

impl GeminiClient {
    pub fn new(api_key: String, model: Option<String>, base_url: Option<String>) -> Self {
        Self {
            api_key,
            model: model.unwrap_or_else(|| "gemini-2.0-flash".to_string()),
            base_url: base_url.unwrap_or_else(|| {
                "https://generativelanguage.googleapis.com/v1beta".to_string()
            }),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(600))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }
}

impl LlmClient for GeminiClient {
    fn complete<'a>(
        &'a self,
        system_prompt: &'a str,
        user_prompt: &'a str,
    ) -> BoxFuture<'a, Result<String, anyhow::Error>> {
        Box::pin(async move {
            // Gemini API 的认证方式是 URL query parameter，而非 Authorization header
            let url = format!(
                "{}/models/{}:generateContent?key={}",
                self.base_url.trim_end_matches('/'),
                self.model,
                self.api_key
            );
            let body = json!({
                "system_instruction": {
                    "parts": [{"text": system_prompt}]
                },
                "contents": [{
                    "parts": [{"text": user_prompt}]
                }],
                "generationConfig": {
                    "temperature": 0.1,
                    "maxOutputTokens": 4096
                }
            });

            let resp = self
                .client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
                .context("Failed to send request to Gemini API")?;

            let status = resp.status();
            if !status.is_success() {
                let err_body = resp.text().await.unwrap_or_default();
                return Err(match status.as_u16() {
                    401 | 403 => {
                        anyhow!("Gemini API authentication failed ({}): invalid API key", status)
                    }
                    429 => anyhow!("Gemini API rate limited (429): {}", err_body),
                    _ => anyhow!("Gemini API error {}: {}", status, err_body),
                });
            }

            let resp_json: Value = resp
                .json()
                .await
                .context("Failed to parse Gemini API response")?;

            // Gemini 响应格式: candidates[0].content.parts[0].text
            resp_json["candidates"][0]["content"]["parts"][0]["text"]
                .as_str()
                .map(|s| s.to_string())
                .ok_or_else(|| {
                    anyhow!(
                        "Unexpected Gemini response format: missing candidates[0].content.parts[0].text"
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
                .ok_or_else(|| anyhow!("Gemini response contained no extractable JSON block"))?;
            serde_json::from_str(json_str).context("Failed to parse JSON from Gemini response")
        })
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}
