/// 每种接口类型的最小可编译参考插件代码。
///
/// 这些模板解决了 LLM 代码生成中最常见的编译失败问题：
/// - 遗漏 use 语句（尤其是 std::collections::HashMap）
/// - export_plugin! 宏调用语法错误
/// - 在 FFI 同步上下文中误用 async/await
///
/// LLM 在生成具体插件代码时以此为骨架，仅需替换业务逻辑部分，
/// 大幅降低生成代码无法编译的概率。
///
/// 注意：SOAP 模板使用 r##"..."## 来容纳内部的 r#"..."# 格式化字符串

pub const SOAP_REFERENCE: &str = r##"
use api_anything_plugin_sdk::*;
use serde::{Serialize, Deserialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize)]
struct SoapRequest {
    // fields derived from WSDL operations
}

#[derive(Serialize, Deserialize)]
struct SoapResponse {
    // fields derived from WSDL response types
}

fn build_soap_envelope(namespace: &str, body_xml: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/" xmlns:ns="{}">
  <soap:Body>{}</soap:Body>
</soap:Envelope>"#,
        namespace, body_xml
    )
}

fn json_to_xml_elements(json: &serde_json::Value) -> String {
    match json {
        serde_json::Value::Object(map) => {
            map.iter().map(|(k, v)| match v {
                serde_json::Value::Object(_) => format!("<{}>{}</{}>", k, json_to_xml_elements(v), k),
                serde_json::Value::Array(arr) => arr.iter()
                    .map(|item| format!("<{}>{}</{}>", k, json_to_xml_elements(item), k))
                    .collect::<Vec<_>>().join(""),
                _ => format!("<{}>{}</{}>", k, v.to_string().trim_matches('"'), k),
            }).collect::<Vec<_>>().join("")
        }
        _ => json.to_string().trim_matches('"').to_string(),
    }
}

fn parse_soap_response(xml: &str) -> serde_json::Value {
    if let Some(start) = xml.find("<soap:Body>") {
        let content_start = start + "<soap:Body>".len();
        if let Some(end) = xml.find("</soap:Body>") {
            let body = &xml[content_start..end];
            let mut result = serde_json::Map::new();
            result.insert("raw".to_string(), serde_json::Value::String(body.to_string()));
            return serde_json::Value::Object(result);
        }
    }
    serde_json::json!({})
}

#[tracing::instrument(skip(req))]
fn handle(req: PluginRequest) -> PluginResponse {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new());

    let body = req.body.unwrap_or(serde_json::json!({}));
    let body_xml = json_to_xml_elements(&body);
    let envelope = build_soap_envelope("http://example.com", &body_xml);

    match client.post("http://example.com/service")
        .header("Content-Type", "text/xml; charset=utf-8")
        .body(envelope)
        .send() {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let body_text = resp.text().unwrap_or_default();
            if status >= 400 {
                PluginResponse {
                    status_code: 502,
                    headers: HashMap::new(),
                    body: serde_json::json!({"error": body_text}),
                }
            } else {
                let parsed = parse_soap_response(&body_text);
                PluginResponse {
                    status_code: 200,
                    headers: HashMap::new(),
                    body: parsed,
                }
            }
        }
        Err(e) => PluginResponse {
            status_code: 502,
            headers: HashMap::new(),
            body: serde_json::json!({"error": format!("{}", e)}),
        },
    }
}

export_plugin!(handle, PluginInfo {
    name: "soap-plugin".to_string(),
    version: "1.0.0".to_string(),
    protocol: "soap".to_string(),
    description: "SOAP service proxy".to_string(),
});
"##;

pub const CLI_REFERENCE: &str = r#"
use api_anything_plugin_sdk::*;
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use std::process::Command;

#[derive(Serialize, Deserialize)]
struct CommandResult {
    exit_code: i32,
    stdout: String,
    stderr: String,
}

fn sanitize_arg(arg: &str) -> Result<String, String> {
    // reject shell metacharacters to prevent injection
    let forbidden = ['|', ';', '&', '$', '`', '(', ')', '{', '}', '<', '>', '\n', '\r'];
    if arg.chars().any(|c| forbidden.contains(&c)) {
        return Err(format!("Argument contains forbidden characters: {}", arg));
    }
    Ok(arg.to_string())
}

#[tracing::instrument(skip(req))]
fn handle(req: PluginRequest) -> PluginResponse {
    let path = req.path.trim_matches('/');
    let body = req.body.unwrap_or(serde_json::json!({}));

    let mut cmd = Command::new("example-tool");
    cmd.arg(path);

    // map JSON body fields to --key value arguments
    if let Some(obj) = body.as_object() {
        for (key, value) in obj {
            let val_str = match value {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            match sanitize_arg(&val_str) {
                Ok(safe_val) => {
                    cmd.arg(format!("--{}", key));
                    cmd.arg(safe_val);
                }
                Err(e) => {
                    return PluginResponse {
                        status_code: 400,
                        headers: HashMap::new(),
                        body: serde_json::json!({"error": e}),
                    };
                }
            }
        }
    }

    match cmd.output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let exit_code = output.status.code().unwrap_or(-1);

            if exit_code != 0 {
                return PluginResponse {
                    status_code: 500,
                    headers: HashMap::new(),
                    body: serde_json::json!({
                        "error": "Command failed",
                        "exit_code": exit_code,
                        "stderr": stderr
                    }),
                };
            }

            // try parsing stdout as JSON, fall back to raw text
            let result_body = serde_json::from_str::<serde_json::Value>(&stdout)
                .unwrap_or(serde_json::json!({"output": stdout.trim()}));

            PluginResponse {
                status_code: 200,
                headers: HashMap::new(),
                body: result_body,
            }
        }
        Err(e) => PluginResponse {
            status_code: 500,
            headers: HashMap::new(),
            body: serde_json::json!({"error": format!("Failed to execute command: {}", e)}),
        },
    }
}

export_plugin!(handle, PluginInfo {
    name: "cli-plugin".to_string(),
    version: "1.0.0".to_string(),
    protocol: "cli".to_string(),
    description: "CLI tool REST wrapper".to_string(),
});
"#;

pub const SSH_REFERENCE: &str = r#"
use api_anything_plugin_sdk::*;
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use std::process::Command;

#[derive(Serialize, Deserialize)]
struct SshConfig {
    host: String,
    user: String,
}

fn sanitize_arg(arg: &str) -> Result<String, String> {
    let forbidden = ['|', ';', '&', '$', '`', '(', ')', '{', '}', '<', '>', '\n', '\r'];
    if arg.chars().any(|c| forbidden.contains(&c)) {
        return Err(format!("Argument contains forbidden characters: {}", arg));
    }
    Ok(arg.to_string())
}

fn execute_ssh_command(host: &str, user: &str, remote_cmd: &str) -> Result<(i32, String, String), String> {
    let output = Command::new("ssh")
        .arg("-o").arg("BatchMode=yes")
        .arg("-o").arg("ConnectTimeout=10")
        .arg("-o").arg("StrictHostKeyChecking=accept-new")
        .arg(format!("{}@{}", user, host))
        .arg(remote_cmd)
        .output()
        .map_err(|e| format!("Failed to execute ssh: {}", e))?;

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    Ok((exit_code, stdout, stderr))
}

#[tracing::instrument(skip(req))]
fn handle(req: PluginRequest) -> PluginResponse {
    let body = req.body.unwrap_or(serde_json::json!({}));
    let host = body.get("host").and_then(|v| v.as_str()).unwrap_or("localhost");
    let user = body.get("user").and_then(|v| v.as_str()).unwrap_or("root");
    let command = body.get("command").and_then(|v| v.as_str()).unwrap_or("");

    if command.is_empty() {
        return PluginResponse {
            status_code: 400,
            headers: HashMap::new(),
            body: serde_json::json!({"error": "Missing 'command' field"}),
        };
    }

    match execute_ssh_command(host, user, command) {
        Ok((exit_code, stdout, stderr)) => {
            if exit_code == 255 {
                PluginResponse {
                    status_code: 502,
                    headers: HashMap::new(),
                    body: serde_json::json!({"error": "SSH connection failed", "stderr": stderr}),
                }
            } else if exit_code != 0 {
                PluginResponse {
                    status_code: 500,
                    headers: HashMap::new(),
                    body: serde_json::json!({
                        "error": "Command failed",
                        "exit_code": exit_code,
                        "stderr": stderr
                    }),
                }
            } else {
                let result_body = serde_json::from_str::<serde_json::Value>(&stdout)
                    .unwrap_or(serde_json::json!({"output": stdout.trim()}));
                PluginResponse {
                    status_code: 200,
                    headers: HashMap::new(),
                    body: result_body,
                }
            }
        }
        Err(e) => PluginResponse {
            status_code: 502,
            headers: HashMap::new(),
            body: serde_json::json!({"error": e}),
        },
    }
}

export_plugin!(handle, PluginInfo {
    name: "ssh-plugin".to_string(),
    version: "1.0.0".to_string(),
    protocol: "ssh".to_string(),
    description: "SSH remote command REST wrapper".to_string(),
});
"#;

pub const PTY_REFERENCE: &str = r#"
use api_anything_plugin_sdk::*;
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::io::Write;

#[derive(Serialize, Deserialize)]
struct PtyCommand {
    input: String,
    expect_prompt: Option<String>,
}

#[tracing::instrument(skip(req))]
fn handle(req: PluginRequest) -> PluginResponse {
    let body = req.body.unwrap_or(serde_json::json!({}));
    let program = body.get("program").and_then(|v| v.as_str()).unwrap_or("sh");
    let commands = body.get("commands").and_then(|v| v.as_array());

    let commands = match commands {
        Some(cmds) => cmds.clone(),
        None => {
            return PluginResponse {
                status_code: 400,
                headers: HashMap::new(),
                body: serde_json::json!({"error": "Missing 'commands' array field"}),
            };
        }
    };

    let mut child = match Command::new(program)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn() {
        Ok(c) => c,
        Err(e) => {
            return PluginResponse {
                status_code: 500,
                headers: HashMap::new(),
                body: serde_json::json!({"error": format!("Failed to spawn process: {}", e)}),
            };
        }
    };

    if let Some(ref mut stdin) = child.stdin {
        for cmd_val in &commands {
            if let Some(cmd_str) = cmd_val.as_str() {
                let _ = writeln!(stdin, "{}", cmd_str);
            }
        }
    }
    // close stdin to signal EOF
    drop(child.stdin.take());

    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => {
            return PluginResponse {
                status_code: 504,
                headers: HashMap::new(),
                body: serde_json::json!({"error": format!("Timeout or IO error: {}", e)}),
            };
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let exit_code = output.status.code().unwrap_or(-1);

    if exit_code != 0 {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        PluginResponse {
            status_code: 500,
            headers: HashMap::new(),
            body: serde_json::json!({
                "error": "Process exited with non-zero status",
                "exit_code": exit_code,
                "stderr": stderr
            }),
        }
    } else {
        PluginResponse {
            status_code: 200,
            headers: HashMap::new(),
            body: serde_json::json!({"output": stdout.trim()}),
        }
    }
}

export_plugin!(handle, PluginInfo {
    name: "pty-plugin".to_string(),
    version: "1.0.0".to_string(),
    protocol: "pty".to_string(),
    description: "Interactive terminal session REST wrapper".to_string(),
});
"#;

pub const ODATA_REFERENCE: &str = r#"
use api_anything_plugin_sdk::*;
use serde::{Serialize, Deserialize};
use std::collections::HashMap;

const BASE_URL: &str = "http://example.com/odata";

#[tracing::instrument(skip(req))]
fn handle(req: PluginRequest) -> PluginResponse {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new());

    let path = req.path.trim_start_matches('/');
    let url = format!("{}/{}", BASE_URL, path);

    // forward OData query parameters ($filter, $select, $top, $skip, $orderby)
    let mut request_url = url.clone();
    if !req.query_params.is_empty() {
        let params: Vec<String> = req.query_params.iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();
        request_url = format!("{}?{}", request_url, params.join("&"));
    }

    let result = match req.method.to_uppercase().as_str() {
        "GET" => client.get(&request_url).send(),
        "POST" => {
            let body = req.body.unwrap_or(serde_json::json!({}));
            client.post(&request_url)
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
        }
        "PATCH" => {
            let body = req.body.unwrap_or(serde_json::json!({}));
            client.patch(&request_url)
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
        }
        "DELETE" => client.delete(&request_url).send(),
        _ => {
            return PluginResponse {
                status_code: 405,
                headers: HashMap::new(),
                body: serde_json::json!({"error": format!("Unsupported method: {}", req.method)}),
            };
        }
    };

    match result {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let body_text = resp.text().unwrap_or_default();
            let body_json = serde_json::from_str::<serde_json::Value>(&body_text)
                .unwrap_or(serde_json::json!({"raw": body_text}));
            PluginResponse {
                status_code: status,
                headers: HashMap::new(),
                body: body_json,
            }
        }
        Err(e) => PluginResponse {
            status_code: 502,
            headers: HashMap::new(),
            body: serde_json::json!({"error": format!("{}", e)}),
        },
    }
}

export_plugin!(handle, PluginInfo {
    name: "odata-plugin".to_string(),
    version: "1.0.0".to_string(),
    protocol: "odata".to_string(),
    description: "OData service proxy".to_string(),
});
"#;

pub const OPENAPI_REFERENCE: &str = r#"
use api_anything_plugin_sdk::*;
use serde::{Serialize, Deserialize};
use std::collections::HashMap;

const BASE_URL: &str = "http://example.com/api";

#[tracing::instrument(skip(req))]
fn handle(req: PluginRequest) -> PluginResponse {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new());

    let path = req.path.trim_start_matches('/');
    let url = format!("{}/{}", BASE_URL, path);

    let mut request_url = url.clone();
    if !req.query_params.is_empty() {
        let params: Vec<String> = req.query_params.iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();
        request_url = format!("{}?{}", request_url, params.join("&"));
    }

    // forward original headers (except hop-by-hop)
    let mut headers = reqwest::header::HeaderMap::new();
    for (key, value) in &req.headers {
        if let (Ok(name), Ok(val)) = (
            reqwest::header::HeaderName::from_bytes(key.as_bytes()),
            reqwest::header::HeaderValue::from_str(value),
        ) {
            headers.insert(name, val);
        }
    }

    let result = match req.method.to_uppercase().as_str() {
        "GET" => client.get(&request_url).headers(headers).send(),
        "POST" => {
            let body = req.body.unwrap_or(serde_json::json!({}));
            client.post(&request_url).headers(headers).json(&body).send()
        }
        "PUT" => {
            let body = req.body.unwrap_or(serde_json::json!({}));
            client.put(&request_url).headers(headers).json(&body).send()
        }
        "PATCH" => {
            let body = req.body.unwrap_or(serde_json::json!({}));
            client.patch(&request_url).headers(headers).json(&body).send()
        }
        "DELETE" => client.delete(&request_url).headers(headers).send(),
        _ => {
            return PluginResponse {
                status_code: 405,
                headers: HashMap::new(),
                body: serde_json::json!({"error": format!("Unsupported method: {}", req.method)}),
            };
        }
    };

    match result {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let mut resp_headers = HashMap::new();
            for (key, value) in resp.headers() {
                if let Ok(v) = value.to_str() {
                    resp_headers.insert(key.to_string(), v.to_string());
                }
            }
            let body_text = resp.text().unwrap_or_default();
            let body_json = serde_json::from_str::<serde_json::Value>(&body_text)
                .unwrap_or(serde_json::json!({"raw": body_text}));
            PluginResponse {
                status_code: status,
                headers: resp_headers,
                body: body_json,
            }
        }
        Err(e) => PluginResponse {
            status_code: 502,
            headers: HashMap::new(),
            body: serde_json::json!({"error": format!("{}", e)}),
        },
    }
}

export_plugin!(handle, PluginInfo {
    name: "openapi-plugin".to_string(),
    version: "1.0.0".to_string(),
    protocol: "rest".to_string(),
    description: "OpenAPI REST service proxy".to_string(),
});
"#;
