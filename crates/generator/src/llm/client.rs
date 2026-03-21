use serde_json::Value;
use std::future::Future;
use std::pin::Pin;

// 使用 BoxFuture 而非 impl Future，确保 trait 可被装入 Box<dyn LlmClient> 做运行时动态分发；
// impl Future 在 trait 方法中会产生 associated type 无法 object-safe 的问题
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait LlmClient: Send + Sync {
    fn complete<'a>(
        &'a self,
        system_prompt: &'a str,
        user_prompt: &'a str,
    ) -> BoxFuture<'a, Result<String, anyhow::Error>>;

    fn complete_json<'a>(
        &'a self,
        system_prompt: &'a str,
        user_prompt: &'a str,
    ) -> BoxFuture<'a, Result<Value, anyhow::Error>>;

    fn model_name(&self) -> &str;
}

/// 从 LLM 响应文本中提取 JSON 块
///
/// LLM 常以 markdown code fence 包裹 JSON，此函数按优先级依次尝试：
/// 1. ```json ... ``` —— 最明确的格式，优先匹配
/// 2. ``` ... ``` —— 通用 fence，需额外检查内容是否以 { 或 [ 开头才接受
/// 3. 裸 JSON —— 无 fence，直接以 { 或 [ 开头的纯文本
///
/// 返回 None 表示文本中不含可解析的 JSON 结构，调用方应视为 LLM 返回无效。
pub fn extract_json_block(text: &str) -> Option<&str> {
    // 尝试 ```json ... ```
    if let Some(start) = text.find("```json") {
        let content_start = start + "```json".len();
        if let Some(end) = text[content_start..].find("```") {
            return Some(text[content_start..content_start + end].trim());
        }
    }

    // 尝试通用 ``` ... ```，需要跳过可选的语言标记行
    if let Some(start) = text.find("```") {
        let content_start = start + "```".len();
        // 跳过紧跟在 ``` 后的语言标识行（如 "rust\n"）
        let content_start = text[content_start..]
            .find('\n')
            .map(|n| content_start + n + 1)
            .unwrap_or(content_start);
        if let Some(end) = text[content_start..].find("```") {
            let candidate = text[content_start..content_start + end].trim();
            // 只接受看起来是 JSON 对象或数组的内容，避免误提取代码块
            if candidate.starts_with('{') || candidate.starts_with('[') {
                return Some(candidate);
            }
        }
    }

    // 尝试裸 JSON：整段文本去空白后直接以 { 或 [ 开头
    let trimmed = text.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return Some(trimmed);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_json_from_code_fence() {
        let text = "Here:\n```json\n{\"name\": \"test\"}\n```\n";
        let json = extract_json_block(text).unwrap();
        let parsed: Value = serde_json::from_str(json).unwrap();
        assert_eq!(parsed["name"], "test");
    }

    #[test]
    fn extracts_raw_json() {
        let text = "{\"name\": \"test\"}";
        let json = extract_json_block(text).unwrap();
        assert_eq!(serde_json::from_str::<Value>(json).unwrap()["name"], "test");
    }

    #[test]
    fn returns_none_for_plain_text() {
        assert!(extract_json_block("just some text").is_none());
    }

    #[test]
    fn extracts_json_array_from_fence() {
        let text = "Result:\n```json\n[{\"id\": 1}, {\"id\": 2}]\n```";
        let json = extract_json_block(text).unwrap();
        let parsed: Value = serde_json::from_str(json).unwrap();
        assert!(parsed.as_array().unwrap().len() == 2);
    }

    #[test]
    fn extracts_raw_json_array() {
        let text = "[{\"id\": 1}]";
        let json = extract_json_block(text).unwrap();
        let parsed: Value = serde_json::from_str(json).unwrap();
        assert!(parsed.as_array().is_some());
    }
}
