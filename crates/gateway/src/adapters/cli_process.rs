use crate::adapter::{BoxFuture, ProtocolAdapter};
use crate::output_parser::{OutputFormat, OutputParser};
use crate::types::*;
use api_anything_common::error::AppError;
use axum::http::{HeaderMap, Method};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Instant;
use tokio::process::Command;

/// CLI 适配器的静态配置；program/subcommand/static_args 在路由注册时确定，
/// 运行期只有来自请求体的动态参数会变化
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliConfig {
    pub program: String,
    pub subcommand: Option<String>,
    /// 每次调用都固定追加的参数，例如 `--output json`
    pub static_args: Vec<String>,
    pub output_format: OutputFormat,
}

pub struct CliAdapter {
    config: CliConfig,
}

impl CliAdapter {
    pub fn new(config: CliConfig) -> Self {
        Self { config }
    }
}

impl ProtocolAdapter for CliAdapter {
    /// 将请求体的 JSON 字段转换为 CLI 参数列表；
    /// bool 值用于处理开关型 flag（true 添加 --flag，false 跳过），
    /// null 值跳过以避免向命令传入字面量 "null" 字符串
    fn transform_request(&self, req: &GatewayRequest) -> Result<BackendRequest, AppError> {
        let mut args: Vec<String> = Vec::new();

        // subcommand 必须排在所有 flag 之前，否则大多数 CLI 工具会解析失败
        if let Some(sub) = &self.config.subcommand {
            args.push(sub.clone());
        }

        args.extend(self.config.static_args.clone());

        // 将 JSON body 的每个字段映射为 --key value 形式
        if let Some(body) = &req.body {
            if let Some(obj) = body.as_object() {
                for (key, val) in obj {
                    match val {
                        serde_json::Value::Bool(true) => {
                            // 布尔 flag 只需 key，无需 value
                            args.push(format!("--{}", key));
                        }
                        serde_json::Value::Bool(false) => {
                            // false 表示不启用该 flag，跳过
                        }
                        serde_json::Value::Null => {
                            // null 表示调用方未传值，跳过以避免命令收到 "null" 字符串
                        }
                        _ => {
                            args.push(format!("--{}", key));
                            args.push(val_to_arg_string(val));
                        }
                    }
                }
            }
        }

        // path params 同样转为 --key value，方便 REST 风格路由复用同一命令
        for (key, val) in &req.path_params {
            args.push(format!("--{}", key));
            args.push(val.clone());
        }

        let mut protocol_params = HashMap::new();
        protocol_params.insert("program".to_string(), self.config.program.clone());
        protocol_params.insert("args".to_string(), args.join(" "));

        Ok(BackendRequest {
            endpoint: self.config.program.clone(),
            method: Method::POST,
            headers: HeaderMap::new(),
            // 将参数列表序列化后存入 body，供 execute() 反序列化使用；
            // 用 JSON 数组而非空格拼接，确保含空格的参数值不被误拆分
            body: Some(serde_json::to_vec(&args).unwrap_or_default()),
            protocol_params,
        })
    }

    fn execute<'a>(
        &'a self,
        req: &'a BackendRequest,
    ) -> BoxFuture<'a, Result<BackendResponse, AppError>> {
        Box::pin(async move {
            let start = Instant::now();

            let args: Vec<String> = req
                .body
                .as_ref()
                .and_then(|b| serde_json::from_slice(b).ok())
                .unwrap_or_default();

            // 安全关键：每个参数独立通过 .arg() 传入，OS 层面保证参数边界，
            // 杜绝 shell 注入——绝不能将参数拼成字符串后交给 shell 执行
            let mut cmd = Command::new(&req.endpoint);
            for arg in &args {
                cmd.arg(arg);
            }

            let output = cmd.output().await.map_err(|e| {
                AppError::BackendUnavailable(format!(
                    "Failed to execute '{}': {e}",
                    req.endpoint
                ))
            })?;

            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let exit_code = output.status.code().unwrap_or(-1);

            let body = serde_json::json!({
                "exit_code": exit_code,
                "stdout": stdout,
                "stderr": stderr,
            });

            Ok(BackendResponse {
                status_code: if exit_code == 0 { 200 } else { 500 },
                headers: HeaderMap::new(),
                body: serde_json::to_vec(&body).unwrap_or_default(),
                is_success: exit_code == 0,
                duration_ms: start.elapsed().as_millis() as u64,
            })
        })
    }

    /// 先检查 exit_code 再解析输出；
    /// 命令失败时 stdout 可能为空或格式混乱，强行解析会产生误导性错误
    fn transform_response(&self, resp: &BackendResponse) -> Result<GatewayResponse, AppError> {
        let raw: serde_json::Value =
            serde_json::from_slice(&resp.body).unwrap_or(serde_json::json!({}));

        let exit_code = raw["exit_code"].as_i64().unwrap_or(-1);
        if exit_code != 0 {
            let stderr = raw["stderr"].as_str().unwrap_or("Unknown error");
            return Err(AppError::BackendError {
                status: 500,
                detail: format!("Command failed (exit code {}): {}", exit_code, stderr),
            });
        }

        let stdout = raw["stdout"].as_str().unwrap_or("");
        let parsed = OutputParser::parse(stdout, &self.config.output_format)?;

        Ok(GatewayResponse {
            status_code: 200,
            headers: HashMap::new(),
            body: parsed,
        })
    }

    fn name(&self) -> &str {
        "cli"
    }
}

/// 将 JSON 值转为命令行参数字符串；
/// String 类型直接取内容避免多余引号，Number 用标准显示，
/// 其他复合类型（Array/Object）序列化为 JSON 字符串作为单个参数
fn val_to_arg_string(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::GatewayRequest;
    use uuid::Uuid;

    fn make_cli_adapter() -> CliAdapter {
        CliAdapter::new(CliConfig {
            program: "echo".to_string(),
            subcommand: None,
            static_args: vec![],
            output_format: OutputFormat::RawText,
        })
    }

    #[test]
    fn transforms_json_to_cli_args() {
        let adapter = make_cli_adapter();
        let req = GatewayRequest {
            route_id: Uuid::new_v4(),
            method: Method::POST,
            path: "/test".to_string(),
            headers: HeaderMap::new(),
            query_params: HashMap::new(),
            path_params: HashMap::new(),
            body: Some(serde_json::json!({"name": "test", "count": 5})),
            trace_id: "t".to_string(),
        };
        let backend_req = adapter.transform_request(&req).unwrap();
        let args: Vec<String> =
            serde_json::from_slice(&backend_req.body.unwrap()).unwrap();
        assert!(args.contains(&"--name".to_string()));
        assert!(args.contains(&"test".to_string()));
        assert!(args.contains(&"--count".to_string()));
    }

    #[tokio::test]
    async fn executes_echo_command() {
        let adapter = CliAdapter::new(CliConfig {
            program: "echo".to_string(),
            subcommand: None,
            static_args: vec!["hello".to_string(), "world".to_string()],
            output_format: OutputFormat::RawText,
        });
        let req = GatewayRequest {
            route_id: Uuid::new_v4(),
            method: Method::POST,
            path: "/test".to_string(),
            headers: HeaderMap::new(),
            query_params: HashMap::new(),
            path_params: HashMap::new(),
            body: None,
            trace_id: "t".to_string(),
        };
        let backend_req = adapter.transform_request(&req).unwrap();
        let backend_resp = adapter.execute(&backend_req).await.unwrap();
        assert!(backend_resp.is_success);

        let gw_resp = adapter.transform_response(&backend_resp).unwrap();
        assert_eq!(gw_resp.status_code, 200);
        let stdout = gw_resp.body["stdout"].as_str().unwrap();
        assert!(stdout.contains("hello"));
        assert!(stdout.contains("world"));
    }

    #[tokio::test]
    async fn handles_nonexistent_command() {
        let adapter = CliAdapter::new(CliConfig {
            program: "/nonexistent/command/xyz".to_string(),
            subcommand: None,
            static_args: vec![],
            output_format: OutputFormat::RawText,
        });
        let req = GatewayRequest {
            route_id: Uuid::new_v4(),
            method: Method::POST,
            path: "/test".to_string(),
            headers: HeaderMap::new(),
            query_params: HashMap::new(),
            path_params: HashMap::new(),
            body: None,
            trace_id: "t".to_string(),
        };
        let backend_req = adapter.transform_request(&req).unwrap();
        let result = adapter.execute(&backend_req).await;
        assert!(result.is_err()); // BackendUnavailable
    }
}
