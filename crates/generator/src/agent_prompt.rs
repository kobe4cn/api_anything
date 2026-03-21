use crate::unified_contract::*;
use crate::shadow_test::ShadowTestGenerator;

pub struct AgentPromptGenerator;

impl AgentPromptGenerator {
    /// 生成结构化 Markdown 提示词，供 AI Agent 理解 API 的操作列表、
    /// 请求/响应 Schema 以及可直接复制执行的 curl 示例
    pub fn generate(contract: &UnifiedContract) -> String {
        let mut prompt = String::new();
        prompt.push_str(&format!("# {} API\n\n", contract.service_name));
        prompt.push_str(&format!("{}\n\n", contract.description));
        prompt.push_str("## Available Operations\n\n");

        for op in &contract.operations {
            prompt.push_str(&format!("### {}\n", op.name));
            prompt.push_str(&format!("- **Method:** {}\n", op.http_method));
            prompt.push_str(&format!("- **Path:** {}\n", op.path));
            prompt.push_str(&format!("- **Description:** {}\n", op.description));

            if let Some(input) = &op.input {
                prompt.push_str("- **Request Body:**\n```json\n");
                prompt.push_str(&serde_json::to_string_pretty(&input.schema).unwrap_or_default());
                prompt.push_str("\n```\n");
            }
            if let Some(output) = &op.output {
                prompt.push_str("- **Response:**\n```json\n");
                prompt.push_str(&serde_json::to_string_pretty(&output.schema).unwrap_or_default());
                prompt.push_str("\n```\n");
            }
            prompt.push('\n');
        }

        // 使用第一个操作生成可直接运行的 curl 示例，降低 Agent 首次调用的摩擦
        prompt.push_str("## Usage Example\n\n");
        if let Some(first_op) = contract.operations.first() {
            let sample_body = first_op.input.as_ref()
                .map(|m| ShadowTestGenerator::generate_sample_from_schema(&m.schema));
            prompt.push_str("```bash\n");
            prompt.push_str(&format!("curl -X {} {{{{base_url}}}}{}", first_op.http_method, first_op.path));
            if let Some(body) = sample_body {
                prompt.push_str(" \\\n  -H 'Content-Type: application/json' \\\n  -d '");
                prompt.push_str(&serde_json::to_string(&body).unwrap_or_default());
                prompt.push('\'');
            }
            prompt.push_str("\n```\n");
        }

        prompt
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wsdl::{parser::WsdlParser, mapper::WsdlMapper};

    fn sample_contract() -> UnifiedContract {
        let wsdl = WsdlParser::parse(include_str!("../tests/fixtures/calculator.wsdl")).unwrap();
        WsdlMapper::map(&wsdl).unwrap()
    }

    #[test]
    fn generates_prompt_with_all_operations() {
        let prompt = AgentPromptGenerator::generate(&sample_contract());
        assert!(prompt.contains("CalculatorService"));
        assert!(prompt.contains("Add"));
        assert!(prompt.contains("GetHistory"));
        assert!(prompt.contains("curl"));
    }

    #[test]
    fn prompt_includes_schema_info() {
        let prompt = AgentPromptGenerator::generate(&sample_contract());
        assert!(prompt.contains("Request Body"));
        assert!(prompt.contains("Response"));
        assert!(prompt.contains("integer")); // from JSON Schema type
    }
}
