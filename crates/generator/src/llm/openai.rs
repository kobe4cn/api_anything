use anyhow::{anyhow, Context};
use serde_json::{json, Value};

use super::client::{extract_json_block, BoxFuture, LlmClient};

pub struct OpenAiClient {
    api_key: String,
    model: String,
    client: reqwest::Client,
}

impl OpenAiClient {
    pub fn new(api_key: String, model: Option<String>) -> Self {
        Self {
            api_key,
            // gpt-4o 是当前 OpenAI 能力与成本的最优选择
            model: model.unwrap_or_else(|| "gpt-4o".to_string()),
            client: reqwest::Client::new(),
        }
    }
}

impl LlmClient for OpenAiClient {
    fn complete<'a>(
        &'a self,
        system_prompt: &'a str,
        user_prompt: &'a str,
    ) -> BoxFuture<'a, Result<String, anyhow::Error>> {
        Box::pin(async move {
            let body = json!({
                "model": self.model,
                "messages": [
                    {"role": "system", "content": system_prompt},
                    {"role": "user", "content": user_prompt}
                ]
            });

            let resp = self
                .client
                .post("https://api.openai.com/v1/chat/completions")
                // OpenAI 使用标准 Bearer token 认证
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
                .context("Failed to send request to OpenAI API")?;

            let status = resp.status();
            // 先检查状态码再解析 body，避免将错误 JSON 体误作成功响应处理
            if !status.is_success() {
                let err_body = resp.text().await.unwrap_or_default();
                return Err(match status.as_u16() {
                    401 => anyhow!("OpenAI API authentication failed (401): invalid API key"),
                    429 => anyhow!("OpenAI API rate limited (429): {}", err_body),
                    _ => anyhow!("OpenAI API error {}: {}", status, err_body),
                });
            }

            let resp_json: Value = resp
                .json()
                .await
                .context("Failed to parse OpenAI API response")?;

            // 从 choices[0].message.content 提取文本，符合 OpenAI Chat Completions API 格式
            resp_json["choices"][0]["message"]["content"]
                .as_str()
                .map(|s| s.to_string())
                .ok_or_else(|| {
                    anyhow!(
                        "Unexpected OpenAI response format: missing choices[0].message.content"
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
                .ok_or_else(|| anyhow!("OpenAI response contained no extractable JSON block"))?;
            serde_json::from_str(json_str).context("Failed to parse JSON from OpenAI response")
        })
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}
