use anyhow::Result;
use serde_json::{json, Value};

use crate::cli_help::parser::{CliDefinition, CliOption};
use crate::unified_contract::{MessageDef, Operation, UnifiedContract};

pub struct CliMapper;

impl CliMapper {
    /// 将解析后的 CLI 定义映射为统一合约，每个子命令对应一个 HTTP 操作。
    /// program_name 用于构造路径前缀和后端绑定中的命令名称。
    pub fn map(def: &CliDefinition, program_name: &str) -> Result<UnifiedContract> {
        let program_slug = to_kebab(program_name);

        let operations = def
            .subcommands
            .iter()
            .map(|sub| {
                let sub_slug = to_kebab(&sub.name);
                let http_method = infer_http_method(&sub.name);
                let path = format!("/api/v1/{}/{}", program_slug, sub_slug);

                let input = if !sub.options.is_empty() {
                    Some(build_request_schema(&sub.name, &sub.options))
                } else {
                    None
                };

                // CLI 操作的响应统一返回 stdout 文本和退出码，
                // 具体结构由 CLI Output Parser（Phase2a-T3）在运行时填充
                let output = Some(MessageDef {
                    name: format!("{}Response", capitalize(&sub.name)),
                    schema: json!({
                        "type": "object",
                        "properties": {
                            "stdout": { "type": "string" },
                            "exit_code": { "type": "integer" }
                        }
                    }),
                });

                Operation {
                    name: sub.name.clone(),
                    description: sub.description.clone(),
                    http_method,
                    path,
                    input,
                    output,
                    // CLI 协议不使用 SOAP action，此字段留空
                    soap_action: None,
                    // endpoint_url 存放程序名，供 CLI 适配器在运行时拼接完整命令
                    endpoint_url: Some(program_name.to_string()),
                }
            })
            .collect();

        Ok(UnifiedContract {
            service_name: program_name.to_string(),
            description: def.description.clone(),
            base_path: format!("/api/v1/{}", program_slug),
            operations,
            types: Vec::new(),
        })
    }
}

/// 根据子命令名称推断 HTTP 方法。
/// 使用动词语义分类而非精确字符串匹配，以覆盖常见命名变体。
fn infer_http_method(name: &str) -> String {
    let lower = name.to_lowercase();
    // 查询类动词：幂等读取，映射为 GET
    if matches!(lower.as_str(), "list" | "show" | "get" | "status" | "info" | "view" | "check" | "describe") {
        return "GET".to_string();
    }
    // 创建类动词：非幂等写入，映射为 POST
    if matches!(lower.as_str(), "create" | "generate" | "add" | "new" | "init") {
        return "POST".to_string();
    }
    // 删除类动词：资源移除，映射为 DELETE
    if matches!(lower.as_str(), "delete" | "remove" | "rm" | "destroy" | "drop") {
        return "DELETE".to_string();
    }
    // 更新类动词：幂等替换，映射为 PUT
    if matches!(lower.as_str(), "update" | "set" | "modify" | "edit" | "change") {
        return "PUT".to_string();
    }
    // 未知动词兜底为 POST，保持与 SOAP 映射器一致的安全默认值
    "POST".to_string()
}

/// 将 CLI 选项列表转换为 JSON Schema 对象，用作请求体 schema。
/// 值类型根据选项的 value_name 占位符推断，以覆盖常见的命令行惯例。
fn build_request_schema(op_name: &str, options: &[CliOption]) -> MessageDef {
    let mut properties = serde_json::Map::new();
    let mut required_fields: Vec<Value> = Vec::new();

    for opt in options {
        // 用长标志名（去掉 "--"）作为 JSON 属性名，与 REST API 命名惯例一致
        let prop_name = opt
            .long
            .as_deref()
            .unwrap_or("")
            .trim_start_matches('-')
            .to_string();

        if prop_name.is_empty() {
            continue;
        }

        let json_type = infer_json_type(opt.value_name.as_deref());

        let mut prop_schema = serde_json::Map::new();
        prop_schema.insert("type".to_string(), json!(json_type));

        if !opt.description.is_empty() {
            prop_schema.insert("description".to_string(), json!(opt.description));
        }

        // 枚举约束优先使用 possible_values，若无则不添加 enum 字段
        if !opt.possible_values.is_empty() {
            let enum_vals: Vec<Value> = opt.possible_values
                .iter()
                .map(|v| json!(v))
                .collect();
            prop_schema.insert("enum".to_string(), json!(enum_vals));
        }

        if let Some(ref default) = opt.default_value {
            prop_schema.insert("default".to_string(), json!(default));
        }

        properties.insert(prop_name.clone(), Value::Object(prop_schema));

        if opt.required {
            required_fields.push(json!(prop_name));
        }
    }

    let mut schema = serde_json::Map::new();
    schema.insert("type".to_string(), json!("object"));
    schema.insert("properties".to_string(), Value::Object(properties));

    if !required_fields.is_empty() {
        schema.insert("required".to_string(), json!(required_fields));
    }

    MessageDef {
        name: format!("{}Request", capitalize(op_name)),
        schema: Value::Object(schema),
    }
}

/// 根据 clap 风格的值占位符名称推断 JSON Schema 类型。
/// 占位符是约定大于配置的命名习惯，如 DATE、PATH、NUM 等。
fn infer_json_type(value_name: Option<&str>) -> &'static str {
    match value_name {
        Some(vn) => {
            let upper = vn.to_uppercase();
            // 数值类占位符
            if matches!(upper.as_str(), "NUMBER" | "COUNT" | "NUM" | "LIMIT" | "PORT") {
                return "integer";
            }
            // 其余占位符（DATE、PATH、TYPE、FORMAT、FILE 等）统一为字符串
            "string"
        }
        // 无值占位符（布尔标志）推断为字符串以保持兼容性
        None => "string",
    }
}

/// 将字符串首字母大写，用于构造类型名（如 "generate" → "Generate"）
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().to_string() + chars.as_str(),
    }
}

/// 将程序名或路径转换为适合 URL 路径段的 kebab-case slug；
/// 当传入绝对路径时提取 basename 并去除扩展名（如 /path/to/mock-report-gen.sh → mock-report-gen），
/// 使生成的 REST 路径不依赖部署环境的目录结构
fn to_kebab(s: &str) -> String {
    // 若字符串包含路径分隔符，则认为是文件路径，取 basename 并去掉扩展名
    let base = if s.contains('/') || s.contains('\\') {
        let stem = std::path::Path::new(s)
            .file_stem()
            .and_then(|os| os.to_str())
            .unwrap_or(s);
        stem
    } else {
        s
    };
    base.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli_help::parser::CliHelpParser;

    fn sample_definition() -> CliDefinition {
        let mut def = CliHelpParser::parse_main(
            include_str!("../../tests/fixtures/sample_help.txt")
        ).unwrap();
        let sub = CliHelpParser::parse_subcommand(
            include_str!("../../tests/fixtures/sample_subcommand_help.txt")
        ).unwrap();
        // 用详细解析结果替换主帮助中的 generate 子命令占位，丰富其 options
        if let Some(gen) = def.subcommands.iter_mut().find(|s| s.name == "generate") {
            gen.options = sub.options;
        }
        def
    }

    #[test]
    fn maps_subcommands_to_operations() {
        let contract = CliMapper::map(&sample_definition(), "report-gen").unwrap();
        assert_eq!(contract.operations.len(), 3); // generate, list, export
    }

    #[test]
    fn infers_http_method_from_name() {
        let contract = CliMapper::map(&sample_definition(), "report-gen").unwrap();
        let list_op = contract.operations.iter().find(|o| o.name == "list").unwrap();
        assert_eq!(list_op.http_method, "GET");
        let gen_op = contract.operations.iter().find(|o| o.name == "generate").unwrap();
        assert_eq!(gen_op.http_method, "POST");
    }

    #[test]
    fn generates_request_schema_from_options() {
        let contract = CliMapper::map(&sample_definition(), "report-gen").unwrap();
        let gen_op = contract.operations.iter().find(|o| o.name == "generate").unwrap();
        let input = gen_op.input.as_ref().unwrap();
        let props = input.schema["properties"].as_object().unwrap();
        assert!(props.contains_key("type"));
        assert!(props.contains_key("format"));
    }

    #[test]
    fn marks_required_options_in_schema() {
        let contract = CliMapper::map(&sample_definition(), "report-gen").unwrap();
        let gen_op = contract.operations.iter().find(|o| o.name == "generate").unwrap();
        let input = gen_op.input.as_ref().unwrap();
        let required = input.schema["required"].as_array().unwrap();
        assert!(required.iter().any(|r| r.as_str() == Some("type")));
    }
}
