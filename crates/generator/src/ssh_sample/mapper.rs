use anyhow::Result;
use serde_json::json;

use crate::ssh_sample::parser::{SshSampleDefinition, SshCommand};
use crate::unified_contract::{MessageDef, Operation, UnifiedContract};

pub struct SshMapper;

impl SshMapper {
    /// 将 SSH 交互样本定义映射为统一合约。
    ///
    /// 每条命令对应一个 HTTP 操作，HTTP 方法由命令动词语义决定，
    /// 路径由 host 和命令模板共同构造，路径参数由 `{param}` 占位符提取。
    pub fn map(def: &SshSampleDefinition) -> Result<UnifiedContract> {
        let host_slug = host_to_slug(&def.host);

        let operations = def.commands.iter()
            .map(|cmd| build_operation(cmd, &host_slug, &def.host, &def.user))
            .collect();

        Ok(UnifiedContract {
            service_name: format!("ssh-{}", host_slug),
            description: def.description.clone(),
            base_path: format!("/api/v1/{}", host_slug),
            operations,
            types: Vec::new(),
        })
    }
}

/// 将单个 SSH 命令映射为统一合约中的一个操作
fn build_operation(
    cmd: &SshCommand,
    host_slug: &str,
    host: &str,
    user: &str,
) -> Operation {
    let http_method = infer_http_method(&cmd.command_template);
    let path = build_path(host_slug, &cmd.command_template);
    let name = command_to_slug(&cmd.command_template);

    // 请求 schema：仅当命令模板中含有路径参数时才构造 input，
    // 无参数的只读命令不需要任何请求体
    let input = if cmd.parameters.is_empty() {
        None
    } else {
        Some(build_request_schema(&cmd.parameters))
    };

    // SSH 命令的响应统一返回原始 stdout，
    // 具体的结构化解析由 SSH 适配器（Phase2b-T3）在运行时负责
    let output = Some(MessageDef {
        name: format!("{}Response", to_pascal(&name)),
        schema: json!({
            "type": "object",
            "properties": {
                "stdout": { "type": "string" }
            }
        }),
    });

    // endpoint_url 编码 SSH 连接目标及命令元信息，
    // 供运行时 SSH 适配器解析后建立连接并执行命令
    let endpoint_config = json!({
        "host": host,
        "user": user,
        "command_template": cmd.command_template,
        "output_format": cmd.output_format,
    });

    Operation {
        name,
        description: cmd.description.clone(),
        http_method,
        path,
        input,
        output,
        soap_action: None,
        endpoint_url: Some(endpoint_config.to_string()),
    }
}

/// 从命令模板的首个动词推断 HTTP 方法。
/// 查询类命令（show/display/get/list）映射为 GET（幂等读取），
/// 配置修改类命令映射为 POST（非幂等写入）。
fn infer_http_method(command_template: &str) -> String {
    let first_word = command_template
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_lowercase();

    match first_word.as_str() {
        "show" | "display" | "get" | "list" => "GET".to_string(),
        "config" | "set" | "enable" | "disable" => "POST".to_string(),
        _ => "POST".to_string(),
    }
}

/// 构造 REST 路径：`/api/v1/{host-slug}/{command-slug}`
/// `{param}` 占位符在路径中保持原样，作为 OpenAPI 路径参数。
fn build_path(host_slug: &str, command_template: &str) -> String {
    let cmd_slug = command_to_slug(command_template);
    format!("/api/v1/{}/{}", host_slug, cmd_slug)
}

/// 将命令模板转换为 URL 路径段：空格替换为短横线，`{param}` 保留。
/// 例如 `show running-config interface {interface}` → `show-running-config-interface/{interface}`
fn command_to_slug(template: &str) -> String {
    // 先把整个模板按空格拆分，再分段处理占位符和普通词
    template
        .split_whitespace()
        .map(|word| {
            if word.starts_with('{') && word.ends_with('}') {
                // 占位符保持原样，供 OpenAPI 路径参数识别
                word.to_string()
            } else {
                word.to_lowercase()
            }
        })
        .collect::<Vec<_>>()
        .join("-")
}

/// 将 IP 地址转换为 URL 友好的 slug，点替换为短横线（10.0.1.50 → 10-0-1-50）
fn host_to_slug(host: &str) -> String {
    host.replace('.', "-")
}

/// 将 kebab-case 名称转换为 PascalCase，用于构造类型名（如 `show-vlan-brief` → `ShowVlanBrief`）
fn to_pascal(s: &str) -> String {
    s.split('-')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().to_string() + chars.as_str(),
            }
        })
        .collect()
}

/// 将路径参数列表构造为 JSON Schema 请求体，每个参数为必填字符串属性
fn build_request_schema(parameters: &[String]) -> MessageDef {
    let mut properties = serde_json::Map::new();
    let required: Vec<serde_json::Value> = parameters
        .iter()
        .map(|p| json!(p))
        .collect();

    for param in parameters {
        properties.insert(param.clone(), json!({
            "type": "string",
            "description": format!("Path parameter: {}", param),
        }));
    }

    MessageDef {
        name: "SshRequest".to_string(),
        schema: json!({
            "type": "object",
            "properties": serde_json::Value::Object(properties),
            "required": required,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh_sample::parser::SshSampleParser;

    #[test]
    fn maps_show_commands_to_get() {
        let def = SshSampleParser::parse(
            include_str!("../../tests/fixtures/ssh_sample.txt")
        ).unwrap();
        let contract = SshMapper::map(&def).unwrap();
        let show_if = contract.operations.iter()
            .find(|o| o.name.contains("show-interfaces"))
            .unwrap();
        assert_eq!(show_if.http_method, "GET");
    }

    #[test]
    fn extracts_path_params() {
        let def = SshSampleParser::parse(
            include_str!("../../tests/fixtures/ssh_sample.txt")
        ).unwrap();
        let contract = SshMapper::map(&def).unwrap();
        let config_cmd = contract.operations.iter()
            .find(|o| o.name.contains("running-config"))
            .unwrap();
        assert!(config_cmd.path.contains("{interface}"));
        let input = config_cmd.input.as_ref().unwrap();
        assert!(input.schema["properties"]["interface"].is_object());
    }

    #[test]
    fn generates_endpoint_config() {
        let def = SshSampleParser::parse(
            include_str!("../../tests/fixtures/ssh_sample.txt")
        ).unwrap();
        let contract = SshMapper::map(&def).unwrap();
        let op = &contract.operations[0];
        // endpoint_url 应包含 SSH 连接目标 host，供运行时适配器解析
        assert!(op.endpoint_url.as_ref().unwrap().contains("10.0.1.50"));
    }
}
