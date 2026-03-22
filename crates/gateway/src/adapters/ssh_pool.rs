//! SSH 连接池模块
//!
//! 基于 russh 纯 Rust SSH 客户端实现，支持连接复用以避免每次命令执行都重新握手。
//! 连接按 "user@host:port" 做 key 分组，每个 key 最多保持 max_per_host 个空闲连接。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use russh::client::{self, Handle};
use russh::keys::{self, PrivateKeyWithHashAlg};
use russh::ChannelMsg;
use tokio::sync::Mutex;

/// 认证方式：密钥文件或密码
#[derive(Debug, Clone)]
pub enum SshAuth {
    KeyFile {
        path: String,
        passphrase: Option<String>,
    },
    Password(String),
    /// 使用系统默认密钥（~/.ssh/id_rsa, ~/.ssh/id_ed25519 等），
    /// 这是网关最常见的场景：运维预先将公钥部署到目标设备
    DefaultKey,
}

/// russh 要求的 client Handler 实现。
/// 仅做最小实现：接受所有服务器公钥（等同于 StrictHostKeyChecking=accept-new），
/// 生产环境应当增加 known_hosts 校验
struct SshClientHandler;

impl client::Handler for SshClientHandler {
    type Error = russh::Error;

    fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::PublicKey,
    ) -> impl std::future::Future<Output = Result<bool, Self::Error>> + Send {
        // 信任所有服务器公钥，与原来 ssh -o StrictHostKeyChecking=accept-new 行为一致
        async { Ok(true) }
    }
}

/// 池中单个 SSH 会话的包装
struct PooledEntry {
    handle: Handle<SshClientHandler>,
    created_at: Instant,
    last_used: Instant,
}

/// SSH 命令执行的输出
#[derive(Debug, Clone)]
pub struct SshCommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// 连接池专用错误类型，与 AppError 解耦便于池内部处理重试逻辑
#[derive(Debug, thiserror::Error)]
pub enum SshPoolError {
    #[error("SSH connection failed: {0}")]
    Connect(String),
    #[error("SSH authentication failed: {0}")]
    Auth(String),
    #[error("Failed to open SSH channel: {0}")]
    ChannelOpen(String),
    #[error("Failed to execute command: {0}")]
    Exec(String),
}

/// SSH 连接池
///
/// 通过复用已认证的 SSH 连接避免重复握手开销（SSH 握手含 key exchange + 认证，
/// 对低性能网络设备可能耗时数秒）。
pub struct SshConnectionPool {
    /// 按 "user@host:port" 分组的空闲连接
    pools: Arc<Mutex<HashMap<String, Vec<PooledEntry>>>>,
    /// 每个 host 最多保持的空闲连接数
    max_per_host: usize,
    /// 连接的最大存活时间，超过后不再复用而是重新建立
    max_lifetime: Duration,
    /// 连接空闲超过此时间后会被清理
    idle_timeout: Duration,
    /// SSH 客户端配置
    ssh_config: Arc<russh::client::Config>,
}

impl SshConnectionPool {
    pub fn new(max_per_host: usize) -> Self {
        let mut config = russh::client::Config::default();
        // 设置 keepalive 以检测断开的连接
        config.keepalive_interval = Some(Duration::from_secs(15));
        config.keepalive_max = 3;
        // TCP_NODELAY 减少小包延迟（SSH 控制消息通常很小）
        config.nodelay = true;

        Self {
            pools: Arc::new(Mutex::new(HashMap::new())),
            max_per_host,
            max_lifetime: Duration::from_secs(300),
            idle_timeout: Duration::from_secs(60),
            ssh_config: Arc::new(config),
        }
    }

    fn pool_key(host: &str, port: u16, user: &str) -> String {
        format!("{}@{}:{}", user, host, port)
    }

    /// 高层 API：从池中获取连接、执行命令、归还连接，一步完成。
    /// 外部无需接触底层 russh Handle，连接生命周期完全由池管理。
    pub async fn execute(
        &self,
        host: &str,
        port: u16,
        user: &str,
        auth: &SshAuth,
        command: &str,
    ) -> Result<SshCommandOutput, SshPoolError> {
        let handle = self.get_or_create(host, port, user, auth).await?;
        let result = Self::run_command(&handle, command).await;

        // 无论命令是否成功，只要连接本身没断就归还到池中
        if !handle.is_closed() {
            self.return_to_pool(host, port, user, handle).await;
        }

        result
    }

    /// 从池中取出或新建一个已认证的 SSH 连接
    async fn get_or_create(
        &self,
        host: &str,
        port: u16,
        user: &str,
        auth: &SshAuth,
    ) -> Result<Handle<SshClientHandler>, SshPoolError> {
        let key = Self::pool_key(host, port, user);

        // 先尝试从池中获取
        if let Some(handle) = self.try_take(&key).await {
            return Ok(handle);
        }

        // 池中无可用连接，新建一个
        self.create_connection(host, port, user, auth).await
    }

    /// 将用完的连接归还到池中供后续复用。
    /// 如果连接已关闭或池已满则直接丢弃。
    async fn return_to_pool(
        &self,
        host: &str,
        port: u16,
        user: &str,
        handle: Handle<SshClientHandler>,
    ) {
        if handle.is_closed() {
            return;
        }

        let key = Self::pool_key(host, port, user);
        let mut pools = self.pools.lock().await;
        let entries = pools.entry(key).or_default();

        if entries.len() < self.max_per_host {
            entries.push(PooledEntry {
                handle,
                created_at: Instant::now(),
                last_used: Instant::now(),
            });
        }
        // 池满则丢弃连接（Drop 时 russh 会自动关闭 TCP）
    }

    /// 在已有连接上执行单条命令并收集 stdout/stderr/exit_code。
    /// 每次 exec 会打开一个新的 SSH channel（SSH 协议允许单连接多 channel）。
    async fn run_command(
        handle: &Handle<SshClientHandler>,
        command: &str,
    ) -> Result<SshCommandOutput, SshPoolError> {
        let mut channel = handle
            .channel_open_session()
            .await
            .map_err(|e| SshPoolError::ChannelOpen(e.to_string()))?;

        channel
            .exec(true, command.as_bytes())
            .await
            .map_err(|e| SshPoolError::Exec(e.to_string()))?;

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit_code: Option<u32> = None;

        // 循环读取 channel 消息，直到远端关闭 channel
        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Data { data } => {
                    stdout.extend_from_slice(&data);
                }
                // ext == 1 是 SSH_EXTENDED_DATA_STDERR（RFC 4254 Section 5.2）
                ChannelMsg::ExtendedData { data, ext: 1 } => {
                    stderr.extend_from_slice(&data);
                }
                ChannelMsg::ExitStatus { exit_status } => {
                    exit_code = Some(exit_status);
                }
                ChannelMsg::Eof | ChannelMsg::Close => break,
                _ => {}
            }
        }

        // 部分设备不发送 ExitStatus（如某些老旧交换机），默认视为成功
        let code = exit_code.unwrap_or(0) as i32;

        Ok(SshCommandOutput {
            stdout: String::from_utf8_lossy(&stdout).to_string(),
            stderr: String::from_utf8_lossy(&stderr).to_string(),
            exit_code: code,
        })
    }

    /// 从池中取出一个健康的连接，跳过已关闭或过期的连接
    async fn try_take(&self, key: &str) -> Option<Handle<SshClientHandler>> {
        let mut pools = self.pools.lock().await;
        let entries = pools.get_mut(key)?;

        while let Some(entry) = entries.pop() {
            let now = Instant::now();

            // 跳过超过最大存活时间的连接（密钥轮换等原因需要定期更新会话）
            if now.duration_since(entry.created_at) > self.max_lifetime {
                continue;
            }

            // 跳过空闲太久的连接（可能已被对端静默关闭）
            if now.duration_since(entry.last_used) > self.idle_timeout {
                continue;
            }

            if entry.handle.is_closed() {
                continue;
            }

            return Some(entry.handle);
        }

        None
    }

    /// 建立新的 SSH 连接并完成认证
    async fn create_connection(
        &self,
        host: &str,
        port: u16,
        user: &str,
        auth: &SshAuth,
    ) -> Result<Handle<SshClientHandler>, SshPoolError> {
        let addr = format!("{}:{}", host, port);
        let mut handle = client::connect(self.ssh_config.clone(), &addr, SshClientHandler)
            .await
            .map_err(|e| SshPoolError::Connect(format!("{}: {}", addr, e)))?;

        // 根据认证方式进行认证
        let auth_result = match auth {
            SshAuth::Password(password) => handle
                .authenticate_password(user, password)
                .await
                .map_err(|e| SshPoolError::Auth(e.to_string()))?,

            SshAuth::KeyFile { path, passphrase } => {
                let key = keys::load_secret_key(path, passphrase.as_deref())
                    .map_err(|e| SshPoolError::Auth(format!("Failed to load key {}: {}", path, e)))?;

                let key_with_hash = PrivateKeyWithHashAlg::new(Arc::new(key), None);
                handle
                    .authenticate_publickey(user, key_with_hash)
                    .await
                    .map_err(|e| SshPoolError::Auth(e.to_string()))?
            }

            SshAuth::DefaultKey => {
                // 按优先级依次尝试常见的默认密钥路径
                let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
                let default_keys = [
                    format!("{}/.ssh/id_ed25519", home),
                    format!("{}/.ssh/id_rsa", home),
                    format!("{}/.ssh/id_ecdsa", home),
                ];

                let mut last_err = String::from("No default SSH key found");
                let mut authenticated = false;

                for key_path in &default_keys {
                    if !Path::new(key_path).exists() {
                        continue;
                    }

                    match keys::load_secret_key(key_path, None) {
                        Ok(key) => {
                            let key_with_hash = PrivateKeyWithHashAlg::new(Arc::new(key), None);
                            match handle.authenticate_publickey(user, key_with_hash).await {
                                Ok(result) if result.success() => {
                                    authenticated = true;
                                    break;
                                }
                                Ok(_) => {
                                    last_err = format!("Key {} rejected by server", key_path);
                                }
                                Err(e) => {
                                    last_err = format!("Auth with {} failed: {}", key_path, e);
                                }
                            }
                        }
                        Err(e) => {
                            last_err = format!("Cannot load {}: {}", key_path, e);
                        }
                    }
                }

                if !authenticated {
                    return Err(SshPoolError::Auth(last_err));
                }

                // DefaultKey 路径已在循环中完成认证校验
                return Ok(handle);
            }
        };

        if !auth_result.success() {
            return Err(SshPoolError::Auth(format!(
                "Authentication failed for {}@{}:{}",
                user, host, port
            )));
        }

        Ok(handle)
    }

    /// 清理过期和关闭的连接，应由后台 task 定期调用
    pub async fn cleanup(&self) {
        let mut pools = self.pools.lock().await;
        let now = Instant::now();

        pools.retain(|_key, entries| {
            entries.retain(|entry| {
                if entry.handle.is_closed() {
                    return false;
                }
                if now.duration_since(entry.created_at) > self.max_lifetime {
                    return false;
                }
                if now.duration_since(entry.last_used) > self.idle_timeout {
                    return false;
                }
                true
            });
            !entries.is_empty()
        });
    }

    /// 启动后台清理任务，每 30 秒清理一次过期连接
    pub fn spawn_cleanup_task(pool: Arc<SshConnectionPool>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                pool.cleanup().await;
                tracing::trace!("SSH connection pool cleanup completed");
            }
        });
    }

    /// 获取当前池中的总连接数（用于监控和测试）
    pub async fn total_connections(&self) -> usize {
        let pools = self.pools.lock().await;
        pools.values().map(|v| v.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_key_format() {
        let key = SshConnectionPool::pool_key("10.0.1.50", 22, "admin");
        assert_eq!(key, "admin@10.0.1.50:22");
    }

    #[test]
    fn pool_key_with_custom_port() {
        let key = SshConnectionPool::pool_key("router.example.com", 2222, "netops");
        assert_eq!(key, "netops@router.example.com:2222");
    }

    #[tokio::test]
    async fn new_pool_is_empty() {
        let pool = SshConnectionPool::new(4);
        assert_eq!(pool.total_connections().await, 0);
    }

    #[tokio::test]
    async fn cleanup_on_empty_pool_is_noop() {
        let pool = SshConnectionPool::new(4);
        pool.cleanup().await;
        assert_eq!(pool.total_connections().await, 0);
    }

    #[test]
    fn ssh_auth_variants() {
        // 确保所有认证方式变体可正常构造
        let _key_auth = SshAuth::KeyFile {
            path: "/home/user/.ssh/id_rsa".to_string(),
            passphrase: None,
        };
        let _pass_auth = SshAuth::Password("secret".to_string());
        let _default = SshAuth::DefaultKey;
    }

    #[test]
    fn pool_default_config() {
        let pool = SshConnectionPool::new(8);
        assert_eq!(pool.max_per_host, 8);
        assert_eq!(pool.max_lifetime, Duration::from_secs(300));
        assert_eq!(pool.idle_timeout, Duration::from_secs(60));
    }
}
