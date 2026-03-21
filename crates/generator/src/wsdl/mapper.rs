use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::unified_contract::{MessageDef, Operation, TypeDef, UnifiedContract};
use super::parser::{WsdlDefinition, WsdlElement};

pub struct WsdlMapper;

impl WsdlMapper {
    pub fn map(wsdl: &WsdlDefinition) -> Result<UnifiedContract> {
        // 服务路径段：去掉 "-service" 后缀再转 kebab，
        // 避免生成 /api/v1/calculator-service/add 这样冗余的路径
        let service_slug = to_kebab_case(&wsdl.service_name);

        let operations = wsdl
            .operations
            .iter()
            .map(|op| {
                let op_slug = to_kebab_case(&op.name);
                let path = format!("/api/v1/{service_slug}/{op_slug}");

                // 解析 input message → element_ref → WsdlType → JSON Schema
                let input = resolve_message(wsdl, &op.input_message)
                    .context(format!("resolve input for {}", op.name))?;
                let output = resolve_message(wsdl, &op.output_message)
                    .context(format!("resolve output for {}", op.name))?;

                Ok(Operation {
                    name: op.name.clone(),
                    description: String::new(),
                    // SOAP 操作统一映射为 HTTP POST，
                    // 因为 SOAP 本身是 RPC over HTTP，没有 REST 语义
                    http_method: "POST".to_string(),
                    path,
                    input,
                    output,
                    soap_action: op.soap_action.clone(),
                    endpoint_url: wsdl.endpoint_url.clone(),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        // 将所有 WsdlType 也导出为 TypeDef，供 OpenAPI 组件区复用
        let types = wsdl
            .types
            .iter()
            .map(|t| TypeDef {
                name: t.name.clone(),
                schema: build_object_schema(&t.elements),
            })
            .collect();

        Ok(UnifiedContract {
            service_name: wsdl.service_name.clone(),
            description: String::new(),
            base_path: format!("/api/v1/{service_slug}"),
            operations,
            types,
        })
    }
}

/// 通过 message name → element_ref → WsdlType 的链式查找生成 JSON Schema，
/// 每一步找不到都以 `None` 优雅降级而非 panic
fn resolve_message(wsdl: &WsdlDefinition, message_name: &str) -> Result<Option<MessageDef>> {
    let msg = match wsdl.messages.iter().find(|m| m.name == message_name) {
        Some(m) => m,
        None => return Ok(None),
    };

    let wsdl_type = match wsdl.types.iter().find(|t| t.name == msg.element_ref) {
        Some(t) => t,
        None => return Ok(None),
    };

    let schema = build_object_schema(&wsdl_type.elements);

    Ok(Some(MessageDef {
        name: wsdl_type.name.clone(),
        schema,
    }))
}

/// 将 XSD sequence 的字段列表转换为 JSON Schema object
fn build_object_schema(elements: &[WsdlElement]) -> Value {
    let mut properties = serde_json::Map::new();
    let mut required: Vec<Value> = Vec::new();

    for elem in elements {
        let base_type = xsd_to_json_type(&elem.type_name);
        let field_schema = if elem.is_array {
            // maxOccurs="unbounded" → JSON Schema array
            json!({ "type": "array", "items": { "type": base_type } })
        } else {
            json!({ "type": base_type })
        };
        properties.insert(elem.name.clone(), field_schema);
        // 所有字段默认视为必填，与 XSD sequence 语义一致
        required.push(Value::String(elem.name.clone()));
    }

    json!({
        "type": "object",
        "properties": properties,
        "required": required,
    })
}

/// 将 XSD 内置类型映射到 JSON Schema 基本类型，
/// 未知类型兜底为 "string" 保证前向兼容
fn xsd_to_json_type(xsd_type: &str) -> &'static str {
    match xsd_type {
        "int" | "integer" | "long" | "short" | "byte"
        | "unsignedInt" | "unsignedLong" | "unsignedShort" | "unsignedByte" => "integer",
        "float" | "double" | "decimal" => "number",
        "boolean" => "boolean",
        _ => "string",
    }
}

/// CamelCase → kebab-case，并去掉末尾的 "-service" 后缀，
/// 例如 "CalculatorService" → "calculator"，"GetHistory" → "get-history"
fn to_kebab_case(s: &str) -> String {
    let mut result = String::new();
    let chars: Vec<char> = s.chars().collect();

    for (i, &ch) in chars.iter().enumerate() {
        if ch.is_uppercase() && i > 0 {
            result.push('-');
        }
        result.push(ch.to_lowercase().next().unwrap());
    }

    // 去掉冗余的 "-service" 后缀，避免路径中出现 /calculator-service/
    result
        .strip_suffix("-service")
        .unwrap_or(&result)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wsdl::parser::WsdlParser;

    fn sample_contract() -> UnifiedContract {
        let wsdl =
            WsdlParser::parse(include_str!("../../tests/fixtures/calculator.wsdl")).unwrap();
        WsdlMapper::map(&wsdl).unwrap()
    }

    #[test]
    fn maps_service_name() {
        let contract = sample_contract();
        assert_eq!(contract.service_name, "CalculatorService");
    }

    #[test]
    fn maps_operations_to_rest_routes() {
        let contract = sample_contract();
        assert_eq!(contract.operations.len(), 2);
        let add = contract.operations.iter().find(|o| o.name == "Add").unwrap();
        assert_eq!(add.http_method, "POST");
        assert!(add.path.contains("add"));
        assert!(add.soap_action.is_some());
    }

    #[test]
    fn generates_json_schema_for_input() {
        let contract = sample_contract();
        let add = contract.operations.iter().find(|o| o.name == "Add").unwrap();
        let input = add.input.as_ref().unwrap();
        let props = input.schema.get("properties").unwrap();
        assert!(props.get("a").is_some());
        assert!(props.get("b").is_some());
    }

    #[test]
    fn maps_xsd_types_to_json_schema_types() {
        let contract = sample_contract();
        let add = contract.operations.iter().find(|o| o.name == "Add").unwrap();
        let input = add.input.as_ref().unwrap();
        let a_type = input.schema["properties"]["a"]["type"].as_str().unwrap();
        assert_eq!(a_type, "integer");
    }

    #[test]
    fn preserves_endpoint_url() {
        let contract = sample_contract();
        let add = contract.operations.iter().find(|o| o.name == "Add").unwrap();
        assert_eq!(
            add.endpoint_url.as_deref(),
            Some("http://example.com/calculator")
        );
    }
}
