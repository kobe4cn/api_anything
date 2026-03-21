use crate::unified_contract::*;
use serde_json::{json, Value};

pub struct OpenApiGenerator;

impl OpenApiGenerator {
    /// 从统一合约生成标准 OpenAPI 3.0.3 规范，
    /// 固定附加 429/502/503/504 响应码，反映网关层面的错误语义而非后端业务错误
    pub fn generate(contract: &UnifiedContract) -> Value {
        let mut paths = serde_json::Map::new();

        for op in &contract.operations {
            let method = op.http_method.to_lowercase();
            let mut operation = serde_json::Map::new();
            operation.insert("operationId".into(), Value::String(op.name.clone()));
            operation.insert("summary".into(), Value::String(op.description.clone()));
            // tags 使用服务名，便于客户端 SDK 生成时按服务分组
            operation.insert("tags".into(), json!([contract.service_name]));

            if let Some(input) = &op.input {
                operation.insert("requestBody".into(), json!({
                    "required": true,
                    "content": { "application/json": { "schema": input.schema } }
                }));
            }

            let mut responses = serde_json::Map::new();
            if let Some(output) = &op.output {
                responses.insert("200".into(), json!({
                    "description": "Successful response",
                    "content": { "application/json": { "schema": output.schema } }
                }));
            } else {
                responses.insert("200".into(), json!({ "description": "Successful response" }));
            }
            // 以下错误码均来自网关层：限流、后端异常、熔断器打开、后端超时
            responses.insert("429".into(), json!({ "description": "Rate limited" }));
            responses.insert("502".into(), json!({ "description": "Backend error" }));
            responses.insert("503".into(), json!({ "description": "Circuit breaker open" }));
            responses.insert("504".into(), json!({ "description": "Backend timeout" }));
            operation.insert("responses".into(), Value::Object(responses));

            let mut method_map = serde_json::Map::new();
            method_map.insert(method, Value::Object(operation));
            paths.insert(op.path.clone(), Value::Object(method_map));
        }

        json!({
            "openapi": "3.0.3",
            "info": {
                "title": format!("{} API", contract.service_name),
                "description": contract.description,
                "version": "1.0.0"
            },
            "paths": paths
        })
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
    fn generates_valid_openapi_structure() {
        let spec = OpenApiGenerator::generate(&sample_contract());
        assert_eq!(spec["openapi"], "3.0.3");
        assert!(spec["info"]["title"].as_str().unwrap().contains("Calculator"));
    }

    #[test]
    fn generates_paths_for_operations() {
        let spec = OpenApiGenerator::generate(&sample_contract());
        assert_eq!(spec["paths"].as_object().unwrap().len(), 2);
    }

    #[test]
    fn includes_request_body_schema() {
        let spec = OpenApiGenerator::generate(&sample_contract());
        let paths = spec["paths"].as_object().unwrap();
        let (_, path_item) = paths.iter().next().unwrap();
        let post = &path_item["post"];
        assert!(post["requestBody"].is_object());
    }

    #[test]
    fn includes_response_schema() {
        let spec = OpenApiGenerator::generate(&sample_contract());
        let paths = spec["paths"].as_object().unwrap();
        let (_, path_item) = paths.iter().next().unwrap();
        let post = &path_item["post"];
        assert!(post["responses"]["200"].is_object());
    }
}
