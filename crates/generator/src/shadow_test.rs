use crate::unified_contract::*;
use serde::{Serialize, Deserialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowTestCase {
    pub name: String,
    pub description: String,
    pub operation: String,
    pub method: String,
    pub path: String,
    pub request_body: Option<Value>,
    pub expected_status: u16,
}

pub struct ShadowTestGenerator;

impl ShadowTestGenerator {
    pub fn generate(contract: &UnifiedContract) -> Vec<ShadowTestCase> {
        let mut tests = Vec::new();
        for op in &contract.operations {
            // 正常请求：用 schema 生成样本数据，用于验证接口在合法输入下的基础行为
            tests.push(ShadowTestCase {
                name: format!("{}_normal", op.name),
                description: format!("Normal request to {}", op.name),
                operation: op.name.clone(),
                method: op.http_method.clone(),
                path: op.path.clone(),
                request_body: op.input.as_ref().map(|m| Self::generate_sample_from_schema(&m.schema)),
                expected_status: 200,
            });

            // 空 body 请求：测试服务在缺少必填字段时的容错行为（应返回 400 或服务定义的错误码）
            // 此处 expected_status 设为 200 是占位符，实际运行时由影子测试引擎根据响应动态判断
            if op.input.is_some() {
                tests.push(ShadowTestCase {
                    name: format!("{}_empty_body", op.name),
                    description: format!("Empty body request to {}", op.name),
                    operation: op.name.clone(),
                    method: op.http_method.clone(),
                    path: op.path.clone(),
                    request_body: Some(json!({})),
                    expected_status: 200,
                });
            }
        }
        tests
    }

    /// 根据 JSON Schema 的 properties 递归生成最小可用样本值，
    /// 每个字段取其类型的典型最小值，用于构造可被服务接受的请求骨架
    pub fn generate_sample_from_schema(schema: &Value) -> Value {
        if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
            let mut obj = serde_json::Map::new();
            for (key, prop) in props {
                let val = match prop.get("type").and_then(|t| t.as_str()) {
                    Some("integer") => json!(1),
                    Some("number") => json!(1.0),
                    Some("boolean") => json!(true),
                    Some("array") => json!([]),
                    // string 及未识别类型统一填 "sample"，保证字段存在即可
                    _ => json!("sample"),
                };
                obj.insert(key.clone(), val);
            }
            Value::Object(obj)
        } else {
            json!({})
        }
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
    fn generates_tests_for_each_operation() {
        let tests = ShadowTestGenerator::generate(&sample_contract());
        // 2 ops × 2 cases (normal + empty_body) = 4
        assert_eq!(tests.len(), 4);
    }

    #[test]
    fn normal_test_has_sample_body() {
        let tests = ShadowTestGenerator::generate(&sample_contract());
        let normal = tests.iter().find(|t| t.name == "Add_normal").unwrap();
        let body = normal.request_body.as_ref().unwrap();
        assert!(body.get("a").is_some());
        assert!(body.get("b").is_some());
    }

    #[test]
    fn sample_from_schema_respects_types() {
        let schema = json!({
            "properties": {
                "count": {"type": "integer"},
                "name": {"type": "string"},
                "active": {"type": "boolean"}
            }
        });
        let sample = ShadowTestGenerator::generate_sample_from_schema(&schema);
        assert!(sample["count"].is_number());
        assert!(sample["name"].is_string());
        assert!(sample["active"].is_boolean());
    }
}
