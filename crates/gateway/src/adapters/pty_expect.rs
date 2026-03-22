use crate::adapter::{BoxFuture, ProtocolAdapter};
use crate::output_parser::{OutputFormat, OutputParser};
use crate::types::*;
use api_anything_common::error::AppError;
use axum::http::{HeaderMap, Method};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

/// PTY 适配器的静态配置；
/// 与 SSH 适配器不同，PTY 直接管理本地子进程的 stdin/stdout，
/// 适合无法通过 ssh -c 单次执行的交互式协议（如设备串口、数据库 REPL）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyConfig {
    pub program: String,
    pub args: Vec<String>,
    /// 提示符正则；用于判断命令已执行完毕、进程等待下一条输入，
    /// 必须足够具体以避免误匹配命令输出内容
    pub prompt_pattern: String,
    /// 命令模板，支持 {param} 占位符替换，例如 "show interface {name}"
    pub command_template: String,
    pub output_format: OutputFormat,
    /// 进程启动后在发送真正命令前需要执行的初始化序列，
    /// 例如登录、切换模式；每条命令发送后等待提示符出现再继续
    pub init_commands: Vec<String>,
    pub timeout_ms: u64,
}

pub struct PtyAdapter {
    config: PtyConfig,
}

impl PtyAdapter {
    pub fn new(config: PtyConfig) -> Self {
        Self { config }
    }

    /// 将命令模板中的 {param} 占位符替换为实际参数值；
    /// 未匹配的占位符保持原样，由调用方保证参数完整性
    fn resolve_command(&self, params: &HashMap<String, String>) -> String {
        let mut cmd = self.config.command_template.clone();
        for (key, val) in params {
            cmd = cmd.replace(&format!("{{{}}}", key), val);
        }
        cmd
    }
}

impl ProtocolAdapter for PtyAdapter {
    /// 合并路径参数和请求体参数，解析命令模板后存入 protocol_params；
    /// 请求体参数可覆盖同名路径参数，使 REST 风格路由与模板参数复用同一逻辑
    fn transform_request(&self, req: &GatewayRequest) -> Result<BackendRequest, AppError> {
        let mut params = req.path_params.clone();
        if let Some(body) = &req.body {
            if let Some(obj) = body.as_object() {
                for (k, v) in obj {
                    params.insert(k.clone(), v.as_str().unwrap_or(&v.to_string()).to_string());
                }
            }
        }

        let command = self.resolve_command(&params);

        let mut protocol_params = HashMap::new();
        protocol_params.insert("command".to_string(), command);

        Ok(BackendRequest {
            endpoint: self.config.program.clone(),
            method: Method::POST,
            headers: HeaderMap::new(),
            body: None,
            protocol_params,
        })
    }

    fn execute<'a>(
        &'a self,
        req: &'a BackendRequest,
    ) -> BoxFuture<'a, Result<BackendResponse, AppError>> {
        Box::pin(async move {
            let start = Instant::now();

            let command = req
                .protocol_params
                .get("command")
                .ok_or_else(|| AppError::Internal("Missing command in protocol_params".into()))?;

            let prompt_re = regex::Regex::new(&self.config.prompt_pattern)
                .map_err(|e| AppError::Internal(format!("Invalid prompt_pattern regex: {e}")))?;

            let timeout = Duration::from_millis(self.config.timeout_ms);

            // 使用 piped stdin/stdout 而非 PTY 设备，
            // 足以支持大多数基于 readline 的交互式程序；
            // 真实 PTY（openpty）仅在程序需要 isatty() 返回 true 时才必须
            let mut child = Command::new(&self.config.program)
                .args(&self.config.args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| {
                    AppError::BackendUnavailable(format!(
                        "Failed to spawn '{}': {e}",
                        self.config.program
                    ))
                })?;

            let stdin = child.stdin.take().ok_or_else(|| {
                AppError::Internal("Failed to acquire child stdin".into())
            })?;
            let stdout = child.stdout.take().ok_or_else(|| {
                AppError::Internal("Failed to acquire child stdout".into())
            })?;

            let mut stdin = tokio::io::BufWriter::new(stdin);
            let mut reader = BufReader::new(stdout);

            // init_commands 用于实现登录序列或模式切换；
            // 每条命令后等待提示符，确保设备处于就绪状态后再发送下一条，
            // 避免命令交错导致输出解析混乱
            for init_cmd in &self.config.init_commands {
                stdin
                    .write_all(format!("{}\n", init_cmd).as_bytes())
                    .await
                    .map_err(|e| AppError::Internal(format!("Write init_command failed: {e}")))?;
                stdin.flush().await
                    .map_err(|e| AppError::Internal(format!("Flush after init_command failed: {e}")))?;

                wait_for_prompt_and_collect(&mut reader, &prompt_re, timeout).await?;
            }

            // 发送实际命令；\n 触发交互式 shell 执行该行
            stdin
                .write_all(format!("{}\n", command).as_bytes())
                .await
                .map_err(|e| AppError::Internal(format!("Write command failed: {e}")))?;
            stdin.flush().await
                .map_err(|e| AppError::Internal(format!("Flush after command failed: {e}")))?;

            let output = wait_for_prompt_and_collect(&mut reader, &prompt_re, timeout).await?;

            // 强制终止子进程；交互式进程不会自行退出，
            // 不 kill 会导致进程泄漏和文件描述符耗尽
            let _ = child.kill().await;

            let body = serde_json::json!({
                "exit_code": 0,
                "stdout": output,
                "stderr": "",
            });

            Ok(BackendResponse {
                status_code: 200,
                headers: HeaderMap::new(),
                body: serde_json::to_vec(&body).unwrap_or_default(),
                is_success: true,
                duration_ms: start.elapsed().as_millis() as u64,
            })
        })
    }

    /// 与 CLI 适配器保持一致：先检查 exit_code，再解析 stdout；
    /// PTY 模式下 exit_code 始终为 0（无法获取交互式命令的退出码），
    /// 错误检测依赖调用方配置正则或业务层校验
    fn transform_response(&self, resp: &BackendResponse) -> Result<GatewayResponse, AppError> {
        let raw: serde_json::Value =
            serde_json::from_slice(&resp.body).unwrap_or(serde_json::json!({}));

        let exit_code = raw["exit_code"].as_i64().unwrap_or(-1);
        if exit_code != 0 {
            let stderr = raw["stderr"].as_str().unwrap_or("Unknown error");
            return Err(AppError::BackendError {
                status: 500,
                detail: format!("PTY command failed (exit code {}): {}", exit_code, stderr),
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
        "pty"
    }
}

/// 持续读取子进程输出直到提示符出现或超时；
/// 超时以整体 deadline 而非单次读取计量，防止缓慢输出被误判为超时；
/// 提示符所在行本身不计入输出，避免将提示符字符串暴露给调用方
async fn wait_for_prompt_and_collect(
    reader: &mut BufReader<tokio::process::ChildStdout>,
    prompt_re: &regex::Regex,
    timeout: Duration,
) -> Result<String, AppError> {
    let mut output = String::new();
    let deadline = Instant::now() + timeout;

    loop {
        if Instant::now() > deadline {
            return Err(AppError::BackendTimeout {
                timeout_ms: timeout.as_millis() as u64,
            });
        }

        let mut line = String::new();
        // 单次读取设 5s 软超时，防止因进程阻塞而永远等待；
        // 超时后继续外层循环以重新检查 deadline，而非立即报错
        let read_result = tokio::time::timeout(
            Duration::from_secs(5),
            reader.read_line(&mut line),
        )
        .await;

        match read_result {
            Ok(Ok(0)) => break, // EOF：子进程关闭了 stdout
            Ok(Ok(_)) => {
                if prompt_re.is_match(&line) {
                    break; // 检测到提示符，命令已执行完毕
                }
                output.push_str(&line);
            }
            Ok(Err(e)) => return Err(AppError::Internal(format!("Read error: {e}"))),
            Err(_) => continue, // 单次读取超时，继续检查 deadline
        }
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn make_pty_adapter() -> PtyAdapter {
        PtyAdapter::new(PtyConfig {
            program: "bash".into(),
            args: vec![],
            prompt_pattern: r"\$\s*$".into(),
            command_template: "show interface {name}".into(),
            output_format: OutputFormat::RawText,
            init_commands: vec![],
            timeout_ms: 5000,
        })
    }

    #[test]
    fn transform_request_resolves_params() {
        let adapter = make_pty_adapter();
        let req = GatewayRequest {
            route_id: Uuid::new_v4(),
            method: Method::GET,
            path: "/test".into(),
            headers: HeaderMap::new(),
            query_params: HashMap::new(),
            path_params: HashMap::from([("name".into(), "eth0".into())]),
            body: None,
            trace_id: "t".into(),
        };
        let backend_req = adapter.transform_request(&req).unwrap();
        assert_eq!(backend_req.protocol_params["command"], "show interface eth0");
    }

    #[test]
    fn transform_request_body_params_override_path_params() {
        // 请求体同名参数应覆盖路径参数，使 REST 路由与精细化请求体复用同一模板
        let adapter = PtyAdapter::new(PtyConfig {
            program: "bash".into(),
            args: vec![],
            prompt_pattern: r"\$".into(),
            command_template: "show interface {name}".into(),
            output_format: OutputFormat::RawText,
            init_commands: vec![],
            timeout_ms: 5000,
        });
        let req = GatewayRequest {
            route_id: Uuid::new_v4(),
            method: Method::POST,
            path: "/test".into(),
            headers: HeaderMap::new(),
            query_params: HashMap::new(),
            path_params: HashMap::from([("name".into(), "eth0".into())]),
            body: Some(serde_json::json!({"name": "eth1"})),
            trace_id: "t".into(),
        };
        let backend_req = adapter.transform_request(&req).unwrap();
        assert_eq!(backend_req.protocol_params["command"], "show interface eth1");
    }

    #[test]
    fn transform_response_handles_pty_output() {
        let adapter = PtyAdapter::new(PtyConfig {
            program: "bash".into(),
            args: vec![],
            prompt_pattern: r"\$".into(),
            command_template: "test".into(),
            output_format: OutputFormat::RawText,
            init_commands: vec![],
            timeout_ms: 5000,
        });
        let resp = BackendResponse {
            status_code: 200,
            headers: HeaderMap::new(),
            body: serde_json::to_vec(&serde_json::json!({
                "exit_code": 0,
                "stdout": "interface eth0 is up",
                "stderr": ""
            }))
            .unwrap(),
            is_success: true,
            duration_ms: 50,
        };
        let gw_resp = adapter.transform_response(&resp).unwrap();
        assert_eq!(gw_resp.status_code, 200);
        assert!(gw_resp.body["stdout"].as_str().unwrap().contains("eth0"));
    }

    #[tokio::test]
    async fn execute_simple_echo_via_bash() {
        // 使用 bash -c 模拟单次命令执行流程：
        // 输出内容后打印 PROMPT> 触发提示符检测，进程随即退出（EOF），
        // 验证基本的 spawn + read + prompt-detect 路径
        let adapter = PtyAdapter::new(PtyConfig {
            program: "bash".into(),
            args: vec![
                "-c".into(),
                "echo 'hello PTY'; echo 'PROMPT>'".into(),
            ],
            prompt_pattern: r"PROMPT>".into(),
            command_template: "unused".into(),
            output_format: OutputFormat::RawText,
            init_commands: vec![],
            timeout_ms: 5000,
        });

        let req = GatewayRequest {
            route_id: Uuid::new_v4(),
            method: Method::GET,
            path: "/test".into(),
            headers: HeaderMap::new(),
            query_params: HashMap::new(),
            path_params: HashMap::new(),
            body: None,
            trace_id: "t".into(),
        };

        let backend_req = adapter.transform_request(&req).unwrap();
        let backend_resp = adapter.execute(&backend_req).await.unwrap();
        assert!(backend_resp.is_success);

        let gw_resp = adapter.transform_response(&backend_resp).unwrap();
        assert_eq!(gw_resp.status_code, 200);
        // 提示符行本身不应出现在输出中，命令输出应正确捕获
        assert!(gw_resp.body["stdout"].as_str().unwrap().contains("hello PTY"));
    }
}
