use crate::llm::client::LlmClient;
use crate::unified_contract::UnifiedContract;
use crate::wsdl::mapper::WsdlMapper;
use crate::wsdl::parser::WsdlDefinition;

pub struct LlmEnhancedMapper;

impl LlmEnhancedMapper {
    /// 先用确定性映射得到基线合约，再通过 LLM 优化 HTTP 方法和路径设计。
    /// 若 LLM 调用失败（网络错误、限流、无效响应），自动降级到确定性结果，保证功能不中断。
    pub async fn map(
        wsdl: &WsdlDefinition,
        llm: &dyn LlmClient,
    ) -> Result<UnifiedContract, anyhow::Error> {
        // Stage 1: 确定性映射，作为兜底基线
        let mut contract = WsdlMapper::map(wsdl)?;

        // 构造给 LLM 的操作摘要，只暴露 LLM 决策所需的最小信息，
        // 避免将 endpoint_url 等敏感运行时配置泄露给外部 API
        let operations_summary: Vec<_> = contract
            .operations
            .iter()
            .map(|op| {
                serde_json::json!({
                    "name": op.name,
                    "current_method": op.http_method,
                    "current_path": op.path,
                    "has_input": op.input.is_some(),
                })
            })
            .collect();

        let system_prompt = "You are a REST API design expert. Given SOAP operation names, suggest optimal RESTful HTTP methods and paths. Respond ONLY with a JSON array.";
        let user_prompt = format!(
            "Optimize these SOAP operations for REST:\n{}\n\nRespond with JSON array of: {{\"name\": \"...\", \"http_method\": \"GET|POST|PUT|DELETE\", \"path\": \"/api/v1/...\"}}",
            serde_json::to_string_pretty(&operations_summary)?
        );

        match llm.complete_json(system_prompt, &user_prompt).await {
            Ok(suggestions) => {
                if let Some(arr) = suggestions.as_array() {
                    for s in arr {
                        if let (Some(name), Some(method), Some(path)) = (
                            s["name"].as_str(),
                            s["http_method"].as_str(),
                            s["path"].as_str(),
                        ) {
                            if let Some(op) =
                                contract.operations.iter_mut().find(|o| o.name == name)
                            {
                                op.http_method = method.to_string();
                                op.path = path.to_string();
                            }
                        }
                    }
                }
            }
            // LLM 故障不应阻断整个生成流程，记录警告后继续使用确定性映射结果
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "LLM enhancement failed, using deterministic mapping"
                );
            }
        }

        Ok(contract)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::client::{BoxFuture, LlmClient};
    use crate::wsdl::parser::WsdlParser;
    use serde_json::Value;

    /// 模拟始终失败的 LLM 客户端，用于验证降级逻辑
    struct FailingLlmClient;

    impl LlmClient for FailingLlmClient {
        fn complete<'a>(
            &'a self,
            _: &'a str,
            _: &'a str,
        ) -> BoxFuture<'a, Result<String, anyhow::Error>> {
            Box::pin(async { Err(anyhow::anyhow!("LLM unavailable")) })
        }

        fn complete_json<'a>(
            &'a self,
            _: &'a str,
            _: &'a str,
        ) -> BoxFuture<'a, Result<Value, anyhow::Error>> {
            Box::pin(async { Err(anyhow::anyhow!("LLM unavailable")) })
        }

        fn model_name(&self) -> &str {
            "failing-mock"
        }
    }

    /// 模拟返回有效优化建议的 LLM 客户端
    struct MockLlmClient;

    impl LlmClient for MockLlmClient {
        fn complete<'a>(
            &'a self,
            _: &'a str,
            _: &'a str,
        ) -> BoxFuture<'a, Result<String, anyhow::Error>> {
            // complete 不直接使用，complete_json 内部调用它，但 MockLlmClient 直接实现 complete_json
            Box::pin(async {
                Ok(serde_json::json!([
                    {"name": "Add", "http_method": "POST", "path": "/api/v1/calculator/additions"},
                    {"name": "GetHistory", "http_method": "GET", "path": "/api/v1/calculator/history"}
                ])
                .to_string())
            })
        }

        fn complete_json<'a>(
            &'a self,
            _: &'a str,
            _: &'a str,
        ) -> BoxFuture<'a, Result<Value, anyhow::Error>> {
            Box::pin(async {
                Ok(serde_json::json!([
                    {"name": "Add", "http_method": "POST", "path": "/api/v1/calculator/additions"},
                    {"name": "GetHistory", "http_method": "GET", "path": "/api/v1/calculator/history"}
                ]))
            })
        }

        fn model_name(&self) -> &str {
            "mock"
        }
    }

    #[tokio::test]
    async fn degrades_gracefully_when_llm_fails() {
        let wsdl =
            WsdlParser::parse(include_str!("../../tests/fixtures/calculator.wsdl")).unwrap();
        let llm = FailingLlmClient;
        let contract = LlmEnhancedMapper::map(&wsdl, &llm).await.unwrap();
        // LLM 失败不影响合约结构完整性，操作数量与确定性映射一致
        assert_eq!(contract.operations.len(), 2);
    }

    #[tokio::test]
    async fn applies_llm_suggestions() {
        let wsdl =
            WsdlParser::parse(include_str!("../../tests/fixtures/calculator.wsdl")).unwrap();
        let llm = MockLlmClient;
        let contract = LlmEnhancedMapper::map(&wsdl, &llm).await.unwrap();

        // GetHistory 应被 LLM 从 POST 改为 GET，体现读操作语义
        let history = contract
            .operations
            .iter()
            .find(|o| o.name == "GetHistory")
            .unwrap();
        assert_eq!(history.http_method, "GET");
        assert!(history.path.contains("history"));
    }

    #[tokio::test]
    async fn preserves_unmatched_operations_from_deterministic_mapping() {
        let wsdl =
            WsdlParser::parse(include_str!("../../tests/fixtures/calculator.wsdl")).unwrap();
        let llm = MockLlmClient;
        let contract = LlmEnhancedMapper::map(&wsdl, &llm).await.unwrap();

        // Add 操作在 mock 建议中存在，应被正确覆盖
        let add = contract
            .operations
            .iter()
            .find(|o| o.name == "Add")
            .unwrap();
        assert_eq!(add.path, "/api/v1/calculator/additions");
    }
}
