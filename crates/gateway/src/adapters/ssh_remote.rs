// ssh_pool 模块独立于 adapters/mod.rs 声明，避免修改 mod.rs 引发并行冲突
#[path = "ssh_pool.rs"]
pub mod ssh_pool;

use crate::adapter::{BoxFuture, ProtocolAdapter};
use crate::output_parser::{OutputFormat, OutputParser};
use ssh_pool::{SshAuth, SshCommandOutput, SshConnectionPool};
use crate::types::*;
use api_anything_common::error::AppError;
use axum::http::{HeaderMap, Method};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    /// 命令模板，支持 {param} 占位符替换，例如 "show interfaces status"
    /// 或 "show running-config interface {interface}"
    pub command_template: String,
    pub output_format: OutputFormat,
    /// SSH 私钥路径；未指定时 ssh 使用系统默认密钥（~/.ssh/id_rsa 等）
    pub identity_file: Option<String>,
}

pub struct SshAdapter {
    config: SshConfig,
    /// 可选的连接池：设置后使用 russh 池化连接，未设置时回退到系统 ssh 命令
    pool: Option<Arc<SshConnectionPool>>,
}

impl SshAdapter {
    pub fn new(config: SshConfig) -> Self {
        Self { config, pool: None }
    }

    /// 创建带连接池的 SshAdapter，启用 russh 原生 SSH 连接复用
    pub fn with_pool(config: SshConfig, pool: Arc<SshConnectionPool>) -> Self {
        Self {
            config,
            pool: Some(pool),
        }
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

    /// 将 SshConfig 中的 identity_file 转换为连接池使用的认证方式
    fn ssh_auth(&self) -> SshAuth {
        match &self.config.identity_file {
            Some(path) => SshAuth::KeyFile {
                path: path.clone(),
                passphrase: None,
            },
            None => SshAuth::DefaultKey,
        }
    }

    /// 使用 russh 连接池执行命令（首选路径）
    async fn execute_via_pool(
        &self,
        pool: &SshConnectionPool,
        command: &str,
    ) -> Result<SshCommandOutput, AppError> {
        let auth = self.ssh_auth();

        pool.execute(
            &self.config.host,
            self.config.port,
            &self.config.user,
            &auth,
            command,
        )
        .await
        .map_err(|e| AppError::BackendUnavailable(format!("SSH pool error: {e}")))
    }

    /// 回退路径：使用系统 ssh 命令执行（与原实现一致）
    async fn execute_via_system_ssh(&self, command: &str) -> Result<SshCommandOutput, AppError> {
        let mut cmd = Command::new("ssh");
        cmd.arg("-o")
            .arg("StrictHostKeyChecking=accept-new")
            .arg("-o")
            .arg("ConnectTimeout=10")
            .arg("-o")
            .arg("BatchMode=yes")
            .arg("-p")
            .arg(self.config.port.to_string());

        if let Some(key_path) = &self.config.identity_file {
            cmd.arg("-i").arg(key_path);
        }

        cmd.arg(format!("{}@{}", self.config.user, self.config.host))
            .arg(command);

        let output = cmd
            .output()
            .await
            .map_err(|e| AppError::BackendUnavailable(format!("SSH execution failed: {e}")))?;

        Ok(SshCommandOutput {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code().unwrap_or(-1),
        })
    }
}

impl ProtocolAdapter for SshAdapter {
    fn transform_request(&self, req: &GatewayRequest) -> Result<BackendRequest, AppError> {
        // 合并路径参数和请求体参数，供命令模板替换使用；
        // 路径参数优先级低于请求体参数，允许请求体覆盖路径中的同名参数
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
        protocol_params.insert("host".to_string(), self.config.host.clone());
        protocol_params.insert("user".to_string(), self.config.user.clone());

        Ok(BackendRequest {
            endpoint: format!("ssh://{}@{}", self.config.user, self.config.host),
            method: Method::POST,
            headers: HeaderMap::new(),
            body: None,
            protocol_params,
        })
    }

    fn execute<'a>(&'a self, req: &'a BackendRequest) -> BoxFuture<'a, Result<BackendResponse, AppError>> {
        Box::pin(async move {
            let start = Instant::now();
            let command = req.protocol_params.get("command")
                .ok_or_else(|| AppError::Internal("Missing command in protocol_params".into()))?;

            // 优先使用 russh 连接池（避免重复 TCP+SSH 握手），
            // 无连接池时回退到系统 ssh 命令
            let output = if let Some(pool) = &self.pool {
                self.execute_via_pool(pool, command).await?
            } else {
                self.execute_via_system_ssh(command).await?
            };

            let exit_code = output.exit_code;

            // SSH 特殊退出码：255 表示 SSH 连接本身失败（非远端命令错误），
            // 将其映射为 502 Bad Gateway 以区别于远端命令失败的 500
            let body = serde_json::json!({
                "exit_code": exit_code,
                "stdout": output.stdout,
                "stderr": output.stderr,
            });

            Ok(BackendResponse {
                status_code: if exit_code == 0 { 200 } else if exit_code == 255 { 502 } else { 500 },
                headers: HeaderMap::new(),
                body: serde_json::to_vec(&body).unwrap_or_default(),
                is_success: exit_code == 0,
                duration_ms: start.elapsed().as_millis() as u64,
            })
        })
    }

    fn transform_response(&self, resp: &BackendResponse) -> Result<GatewayResponse, AppError> {
        let raw: serde_json::Value = serde_json::from_slice(&resp.body)
            .unwrap_or(serde_json::json!({}));

        let exit_code = raw["exit_code"].as_i64().unwrap_or(-1);

        // SSH 连接层错误（exit 255）与远端命令错误分开处理，
        // 前者是基础设施问题，后者是业务逻辑问题，客户端重试策略不同
        if exit_code == 255 {
            let stderr = raw["stderr"].as_str().unwrap_or("SSH connection failed");
            return Err(AppError::BackendUnavailable(
                format!("SSH connection error: {}", stderr)
            ));
        }

        if exit_code != 0 {
            let stderr = raw["stderr"].as_str().unwrap_or("Unknown error");
            return Err(AppError::BackendError {
                status: 500,
                detail: format!("SSH command failed (exit code {}): {}", exit_code, stderr),
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

    fn name(&self) -> &str { "ssh" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn make_ssh_adapter() -> SshAdapter {
        SshAdapter::new(SshConfig {
            host: "10.0.1.50".to_string(),
            port: 22,
            user: "admin".to_string(),
            command_template: "show interfaces status".to_string(),
            output_format: OutputFormat::RawText,
            identity_file: None,
        })
    }

    #[test]
    fn transform_request_builds_ssh_command() {
        let adapter = make_ssh_adapter();
        let req = GatewayRequest {
            route_id: Uuid::new_v4(),
            method: Method::GET,
            path: "/test".to_string(),
            headers: HeaderMap::new(),
            query_params: HashMap::new(),
            path_params: HashMap::new(),
            body: None,
            trace_id: "t".to_string(),
        };
        let backend_req = adapter.transform_request(&req).unwrap();
        assert_eq!(backend_req.protocol_params["command"], "show interfaces status");
        assert!(backend_req.endpoint.contains("10.0.1.50"));
    }

    #[test]
    fn transform_request_substitutes_params() {
        let adapter = SshAdapter::new(SshConfig {
            host: "10.0.1.50".to_string(),
            port: 22,
            user: "admin".to_string(),
            command_template: "show running-config interface {interface}".to_string(),
            output_format: OutputFormat::RawText,
            identity_file: None,
        });
        let req = GatewayRequest {
            route_id: Uuid::new_v4(),
            method: Method::GET,
            path: "/test".to_string(),
            headers: HeaderMap::new(),
            query_params: HashMap::new(),
            path_params: HashMap::from([("interface".into(), "Gi0/1".into())]),
            body: None,
            trace_id: "t".to_string(),
        };
        let backend_req = adapter.transform_request(&req).unwrap();
        assert_eq!(backend_req.protocol_params["command"], "show running-config interface Gi0/1");
    }

    #[test]
    fn transform_response_parses_successful_output() {
        let adapter = make_ssh_adapter();
        let resp = BackendResponse {
            status_code: 200,
            headers: HeaderMap::new(),
            body: serde_json::to_vec(&serde_json::json!({
                "exit_code": 0,
                "stdout": "Port Gi0/1 connected",
                "stderr": ""
            })).unwrap(),
            is_success: true,
            duration_ms: 50,
        };
        let gw_resp = adapter.transform_response(&resp).unwrap();
        assert_eq!(gw_resp.status_code, 200);
        // RawText 格式将 stdout 包装为 {"stdout": "..."} 形式
        assert_eq!(gw_resp.body["stdout"], "Port Gi0/1 connected");
    }
}
