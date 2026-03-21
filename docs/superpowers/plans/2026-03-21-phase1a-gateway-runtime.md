# Phase 1a: 网关运行时核心 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 构建网关运行时的核心基础设施 — ProtocolAdapter trait、后端保护层（熔断/限流/信号量）、动态路由器、后端调度器和错误规范化引擎，为后续 SOAP/CLI/SSH 适配器的接入提供可运行的运行时框架。

**Architecture:** 新建 `gateway` crate 封装所有数据平面逻辑。ProtocolAdapter trait 定义统一的协议适配接口，BackendDispatcher 编排适配器调用并施加保护策略（限流→信号量→熔断→超时），DynamicRouter 从元数据仓库加载路由表并通过 RCU 原子更新实现热加载。gateway crate 最终挂载为 platform-api 的子路由。

**Tech Stack:** Rust, Axum 0.8, tokio (Semaphore + time), arc-swap (RCU), reqwest (HTTP 连接池), sqlx

**Spec:** `docs/superpowers/specs/2026-03-21-api-anything-platform-design.md` §5

---

## File Structure

```
crates/gateway/
├── Cargo.toml
└── src/
    ├── lib.rs                      # 重导出 + build_gateway_router()
    ├── types.rs                    # GatewayRequest, BackendRequest, BackendResponse, GatewayResponse
    ├── adapter.rs                  # ProtocolAdapter trait
    ├── protection/
    │   ├── mod.rs
    │   ├── rate_limiter.rs         # 令牌桶限流器
    │   ├── circuit_breaker.rs      # 滑动窗口熔断器 (Closed/Open/HalfOpen)
    │   └── semaphore.rs            # 并发信号量包装
    ├── error_normalizer.rs         # 后端错误 → RFC 7807 转换
    ├── dispatcher.rs               # BackendDispatcher: 编排 adapter + protection
    └── router.rs                   # DynamicRouter: 元数据路由表 + RCU 热加载
```

同时修改:
- `Cargo.toml` (workspace) — 添加 gateway 成员 + 新依赖
- `crates/metadata/src/repo.rs` — 扩展 MetadataRepo trait 增加路由查询方法
- `crates/metadata/src/pg.rs` — 实现路由查询
- `crates/platform-api/src/lib.rs` — 挂载 gateway 路由

---

### Task 1: Gateway Crate 脚手架 + 核心类型

**Files:**
- Create: `crates/gateway/Cargo.toml`
- Create: `crates/gateway/src/lib.rs`
- Create: `crates/gateway/src/types.rs`
- Create: `crates/gateway/src/adapter.rs`
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: 添加 gateway 到 workspace**

在根 `Cargo.toml` 的 `members` 中添加 `"crates/gateway"`，并添加新的 workspace 依赖：

```toml
arc-swap = "1"
dashmap = "6"
```

- [ ] **Step 2: 创建 crates/gateway/Cargo.toml**

```toml
[package]
name = "api-anything-gateway"
version.workspace = true
edition.workspace = true

[dependencies]
api-anything-common = { path = "../common" }
api-anything-metadata = { path = "../metadata" }
axum.workspace = true
tokio.workspace = true
serde.workspace = true
serde_json.workspace = true
uuid.workspace = true
chrono.workspace = true
tracing.workspace = true
thiserror.workspace = true
arc-swap.workspace = true
dashmap.workspace = true
reqwest = { version = "0.12", features = ["json"] }

[dev-dependencies]
tokio = { workspace = true, features = ["test-util"] }
assert_json_diff.workspace = true
```

- [ ] **Step 3: 创建 types.rs — 网关请求/响应类型**

```rust
use axum::http::{HeaderMap, Method};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

/// 网关接收的外部请求（经中间件处理后）
#[derive(Debug, Clone)]
pub struct GatewayRequest {
    pub route_id: Uuid,
    pub method: Method,
    pub path: String,
    pub headers: HeaderMap,
    pub query_params: HashMap<String, String>,
    pub path_params: HashMap<String, String>,
    pub body: Option<Value>,
    pub trace_id: String,
}

/// 发送给后端系统的请求（经 Plugin transform_request 转换后）
#[derive(Debug, Clone)]
pub struct BackendRequest {
    pub endpoint: String,
    pub method: Method,
    pub headers: HeaderMap,
    pub body: Option<Vec<u8>>,
    /// 协议特定的额外参数（如 SOAP Action、CLI args 等）
    pub protocol_params: HashMap<String, String>,
}

/// 从后端系统收到的原始响应
#[derive(Debug, Clone)]
pub struct BackendResponse {
    pub status_code: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    /// 后端执行是否成功（用于熔断器统计）
    pub is_success: bool,
    pub duration_ms: u64,
}

/// 网关返回给客户端的最终响应（经 Plugin transform_response 转换后）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayResponse {
    pub status_code: u16,
    pub headers: HashMap<String, String>,
    pub body: Value,
}
```

- [ ] **Step 4: 创建 adapter.rs — ProtocolAdapter trait**

```rust
use crate::types::{BackendRequest, BackendResponse, GatewayRequest, GatewayResponse};
use api_anything_common::error::AppError;

/// 所有协议适配器的统一接口
/// 每种后端协议（SOAP/HTTP/CLI/SSH/PTY）实现此 trait
pub trait ProtocolAdapter: Send + Sync {
    /// 将网关请求转换为后端请求格式
    fn transform_request(
        &self,
        req: &GatewayRequest,
    ) -> Result<BackendRequest, AppError>;

    /// 执行后端调用
    fn execute(
        &self,
        req: &BackendRequest,
    ) -> impl std::future::Future<Output = Result<BackendResponse, AppError>> + Send;

    /// 将后端响应转换为网关标准响应
    fn transform_response(
        &self,
        resp: &BackendResponse,
    ) -> Result<GatewayResponse, AppError>;

    /// 适配器名称（用于日志和监控）
    fn name(&self) -> &str;
}
```

- [ ] **Step 5: 创建 lib.rs**

```rust
pub mod adapter;
pub mod types;

// 后续 Task 逐步添加:
// pub mod protection;
// pub mod error_normalizer;
// pub mod dispatcher;
// pub mod router;
```

- [ ] **Step 6: 验证编译**

Run: `cargo check --workspace`
Expected: 编译成功

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(gateway): add gateway crate with ProtocolAdapter trait and core types"
```

---

### Task 2: 令牌桶限流器 (Rate Limiter)

**Files:**
- Create: `crates/gateway/src/protection/mod.rs`
- Create: `crates/gateway/src/protection/rate_limiter.rs`

- [ ] **Step 1: 编写限流器测试**

在 `rate_limiter.rs` 底部写内联测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn allows_requests_within_limit() {
        let limiter = RateLimiter::new(10, 10); // 10 rps, burst 10
        for _ in 0..10 {
            assert!(limiter.try_acquire().is_ok());
        }
    }

    #[tokio::test]
    async fn rejects_requests_over_limit() {
        let limiter = RateLimiter::new(2, 2); // 2 rps, burst 2
        assert!(limiter.try_acquire().is_ok());
        assert!(limiter.try_acquire().is_ok());
        assert!(limiter.try_acquire().is_err()); // 第3个被拒
    }

    #[tokio::test]
    async fn refills_tokens_over_time() {
        let limiter = RateLimiter::new(100, 1); // 100 rps, burst 1
        assert!(limiter.try_acquire().is_ok());
        assert!(limiter.try_acquire().is_err());
        tokio::time::sleep(Duration::from_millis(15)).await;
        assert!(limiter.try_acquire().is_ok()); // 令牌已补充
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p api-anything-gateway`
Expected: FAIL

- [ ] **Step 3: 实现 RateLimiter**

```rust
use std::sync::Mutex;
use std::time::{Duration, Instant};
use api_anything_common::error::AppError;

/// 令牌桶限流器
/// 按固定速率补充令牌，burst 控制桶容量
pub struct RateLimiter {
    inner: Mutex<TokenBucket>,
}

struct TokenBucket {
    tokens: f64,
    max_tokens: f64,
    refill_rate: f64,  // 每秒补充的令牌数
    last_refill: Instant,
}

impl RateLimiter {
    pub fn new(requests_per_second: u32, burst_size: u32) -> Self {
        Self {
            inner: Mutex::new(TokenBucket {
                tokens: burst_size as f64,
                max_tokens: burst_size as f64,
                refill_rate: requests_per_second as f64,
                last_refill: Instant::now(),
            }),
        }
    }

    /// 尝试获取一个令牌，成功返回 Ok，被限流返回 Err
    pub fn try_acquire(&self) -> Result<(), AppError> {
        let mut bucket = self.inner.lock().unwrap();
        bucket.refill();

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            Err(AppError::RateLimited)
        }
    }
}

impl TokenBucket {
    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.max_tokens);
        self.last_refill = now;
    }
}
```

注意：需要在 `api_anything_common::error::AppError` 中添加 `RateLimited` 变体：

```rust
#[error("Rate limited")]
RateLimited,
```

对应的 IntoResponse 实现中添加：

```rust
AppError::RateLimited => {
    ProblemDetail {
        error_type: "about:blank".to_string(),
        title: "Too Many Requests".to_string(),
        status: 429,
        detail: Some("Rate limit exceeded".to_string()),
        instance: None,
    }.into_response()
}
```

- [ ] **Step 4: 创建 protection/mod.rs**

注意：只暴露已实现的模块，后续 Task 逐步添加。

```rust
pub mod rate_limiter;

pub use rate_limiter::RateLimiter;
```

创建空的 `circuit_breaker.rs` 和 `semaphore.rs` 文件（仅占位，不在 mod.rs 中引用）。Task 3 和 Task 4 完成后分别添加 `pub mod` 和 `pub use`。

- [ ] **Step 5: 更新 gateway lib.rs 添加 protection 模块**

- [ ] **Step 6: 运行测试确认通过**

Run: `cargo test -p api-anything-gateway`
Expected: 3 tests PASS

- [ ] **Step 7: Commit**

```bash
git commit -am "feat(gateway): add token bucket rate limiter"
```

---

### Task 3: 滑动窗口熔断器 (Circuit Breaker)

**Files:**
- Modify: `crates/gateway/src/protection/circuit_breaker.rs`

- [ ] **Step 1: 编写熔断器测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_in_closed_state() {
        let cb = CircuitBreaker::new(50.0, Duration::from_secs(30), Duration::from_secs(60), 3);
        assert_eq!(cb.state(), CircuitState::Closed);
    }

    #[test]
    fn opens_when_error_rate_exceeds_threshold() {
        let cb = CircuitBreaker::new(50.0, Duration::from_secs(30), Duration::from_secs(60), 3);
        // 记录10次失败，0次成功 → 100% 错误率，超过 50% 阈值
        for _ in 0..10 {
            cb.record_failure();
        }
        assert_eq!(cb.state(), CircuitState::Open);
    }

    #[test]
    fn stays_closed_when_error_rate_below_threshold() {
        let cb = CircuitBreaker::new(50.0, Duration::from_secs(30), Duration::from_secs(60), 3);
        // 3次失败 + 7次成功 = 30% 错误率，低于 50%
        for _ in 0..3 { cb.record_failure(); }
        for _ in 0..7 { cb.record_success(); }
        assert_eq!(cb.state(), CircuitState::Closed);
    }

    #[test]
    fn allows_check_returns_false_when_open() {
        let cb = CircuitBreaker::new(50.0, Duration::from_secs(30), Duration::from_secs(60), 3);
        for _ in 0..10 { cb.record_failure(); }
        assert!(!cb.allow_request());
    }

    #[test]
    fn rejects_when_open_then_transitions_to_half_open() {
        let cb = CircuitBreaker::new(50.0, Duration::from_secs(30), Duration::from_millis(50), 3);
        for _ in 0..10 { cb.record_failure(); }
        assert_eq!(cb.state(), CircuitState::Open);

        // 等待 open_duration 过期
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(cb.state(), CircuitState::HalfOpen);
        assert!(cb.allow_request()); // half-open 允许试探性请求
    }

    #[test]
    fn half_open_closes_on_success() {
        let cb = CircuitBreaker::new(50.0, Duration::from_secs(30), Duration::from_millis(50), 1);
        for _ in 0..10 { cb.record_failure(); }
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(cb.state(), CircuitState::HalfOpen);

        cb.record_success(); // half-open 下成功 → 关闭
        assert_eq!(cb.state(), CircuitState::Closed);
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

- [ ] **Step 3: 实现 CircuitBreaker**

```rust
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

/// 滑动窗口熔断器
/// 在窗口内统计错误率，超过阈值则打开熔断
pub struct CircuitBreaker {
    inner: Mutex<CircuitBreakerInner>,
}

struct CircuitBreakerInner {
    error_threshold_percent: f64,
    window_duration: Duration,
    open_duration: Duration,
    half_open_max_requests: u32,

    // 滑动窗口内的统计
    successes: u32,
    failures: u32,
    window_start: Instant,

    // 状态
    state: CircuitState,
    opened_at: Option<Instant>,
    half_open_successes: u32,
}

impl CircuitBreaker {
    pub fn new(
        error_threshold_percent: f64,
        window_duration: Duration,
        open_duration: Duration,
        half_open_max_requests: u32,
    ) -> Self {
        Self {
            inner: Mutex::new(CircuitBreakerInner {
                error_threshold_percent,
                window_duration,
                open_duration,
                half_open_max_requests,
                successes: 0,
                failures: 0,
                window_start: Instant::now(),
                state: CircuitState::Closed,
                opened_at: None,
                half_open_successes: 0,
            }),
        }
    }

    pub fn state(&self) -> CircuitState {
        let mut inner = self.inner.lock().unwrap();
        inner.check_state_transition();
        inner.state
    }

    pub fn allow_request(&self) -> bool {
        let mut inner = self.inner.lock().unwrap();
        inner.check_state_transition();
        match inner.state {
            CircuitState::Closed => true,
            CircuitState::Open => false,
            CircuitState::HalfOpen => true, // 允许试探性请求
        }
    }

    pub fn record_success(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.check_state_transition();
        match inner.state {
            CircuitState::Closed => {
                inner.maybe_reset_window();
                inner.successes += 1;
            }
            CircuitState::HalfOpen => {
                inner.half_open_successes += 1;
                if inner.half_open_successes >= inner.half_open_max_requests {
                    // half-open 下足够多的成功 → 关闭
                    inner.state = CircuitState::Closed;
                    inner.reset_counters();
                }
            }
            CircuitState::Open => {} // open 状态下不应有请求到达
        }
    }

    pub fn record_failure(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.check_state_transition();
        match inner.state {
            CircuitState::Closed => {
                inner.maybe_reset_window();
                inner.failures += 1;
                // 检查是否需要打开熔断
                let total = inner.successes + inner.failures;
                if total > 0 {
                    let error_rate = (inner.failures as f64 / total as f64) * 100.0;
                    if error_rate >= inner.error_threshold_percent {
                        inner.state = CircuitState::Open;
                        inner.opened_at = Some(Instant::now());
                        tracing::warn!(
                            error_rate = error_rate,
                            threshold = inner.error_threshold_percent,
                            "Circuit breaker opened"
                        );
                    }
                }
            }
            CircuitState::HalfOpen => {
                // half-open 下失败 → 重新打开
                inner.state = CircuitState::Open;
                inner.opened_at = Some(Instant::now());
            }
            CircuitState::Open => {}
        }
    }
}

impl CircuitBreakerInner {
    fn check_state_transition(&mut self) {
        if self.state == CircuitState::Open {
            if let Some(opened_at) = self.opened_at {
                if opened_at.elapsed() >= self.open_duration {
                    self.state = CircuitState::HalfOpen;
                    self.half_open_successes = 0;
                }
            }
        }
    }

    fn maybe_reset_window(&mut self) {
        if self.window_start.elapsed() >= self.window_duration {
            self.reset_counters();
        }
    }

    fn reset_counters(&mut self) {
        self.successes = 0;
        self.failures = 0;
        self.window_start = Instant::now();
        self.half_open_successes = 0;
    }
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p api-anything-gateway`
Expected: rate_limiter 3 + circuit_breaker 6 = 9 tests PASS

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(gateway): add sliding window circuit breaker"
```

---

### Task 4: 并发信号量 (Concurrency Semaphore)

**Files:**
- Modify: `crates/gateway/src/protection/semaphore.rs`

- [ ] **Step 1: 编写信号量测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn acquires_within_limit() {
        let sem = ConcurrencySemaphore::new(3);
        let _p1 = sem.acquire().await.unwrap();
        let _p2 = sem.acquire().await.unwrap();
        let _p3 = sem.acquire().await.unwrap();
        assert_eq!(sem.available_permits(), 0);
    }

    #[tokio::test]
    async fn releases_on_drop() {
        let sem = ConcurrencySemaphore::new(1);
        {
            let _permit = sem.acquire().await.unwrap();
            assert_eq!(sem.available_permits(), 0);
        }
        // permit dropped
        assert_eq!(sem.available_permits(), 1);
    }

    #[tokio::test]
    async fn try_acquire_fails_when_exhausted() {
        let sem = ConcurrencySemaphore::new(1);
        let _p1 = sem.acquire().await.unwrap();
        assert!(sem.try_acquire().is_err());
    }
}
```

- [ ] **Step 2: 实现 ConcurrencySemaphore**

```rust
use std::sync::Arc;
use tokio::sync::{Semaphore, OwnedSemaphorePermit};
use api_anything_common::error::AppError;

/// 并发信号量 — 限制同时访问后端的并发数
/// 每个 BackendBinding 独立配置
pub struct ConcurrencySemaphore {
    semaphore: Arc<Semaphore>,
    max_permits: u32,
}

impl ConcurrencySemaphore {
    pub fn new(max_concurrent: u32) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent as usize)),
            max_permits: max_concurrent,
        }
    }

    /// 异步等待获取许可（会阻塞直到有可用许可）
    pub async fn acquire(&self) -> Result<OwnedSemaphorePermit, AppError> {
        self.semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| AppError::Internal("Semaphore closed".to_string()))
    }

    /// 尝试立即获取许可，无可用许可时返回错误
    pub fn try_acquire(&self) -> Result<OwnedSemaphorePermit, AppError> {
        self.semaphore
            .clone()
            .try_acquire_owned()
            .map_err(|_| AppError::Internal("Concurrency limit reached".to_string()))
    }

    pub fn available_permits(&self) -> u32 {
        self.semaphore.available_permits() as u32
    }
}
```

- [ ] **Step 3: 运行测试确认通过**

Run: `cargo test -p api-anything-gateway`
Expected: 12 tests PASS (3 rate_limiter + 6 circuit_breaker + 3 semaphore)

- [ ] **Step 4: Commit**

```bash
git commit -am "feat(gateway): add concurrency semaphore"
```

---

### Task 5: 错误规范化引擎 (Error Normalizer)

**Files:**
- Create: `crates/gateway/src/error_normalizer.rs`

- [ ] **Step 1: 编写错误规范化测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_http_error_status() {
        let resp = BackendResponse {
            status_code: 500,
            headers: HeaderMap::new(),
            body: b"Internal Server Error".to_vec(),
            is_success: false,
            duration_ms: 100,
        };
        let result = ErrorNormalizer::normalize(&resp);
        assert!(result.is_err());
        // 应返回 AppError 包含 ProblemDetail 信息
    }

    #[test]
    fn passes_through_successful_response() {
        let resp = BackendResponse {
            status_code: 200,
            headers: HeaderMap::new(),
            body: b"OK".to_vec(),
            is_success: true,
            duration_ms: 50,
        };
        let result = ErrorNormalizer::normalize(&resp);
        assert!(result.is_ok());
    }

    #[test]
    fn normalizes_soap_fault() {
        let soap_fault = r#"<soap:Fault><faultcode>soap:Server</faultcode><faultstring>Order not found</faultstring></soap:Fault>"#;
        let resp = BackendResponse {
            status_code: 200, // SOAP 错误常返回 200
            headers: HeaderMap::new(),
            body: soap_fault.as_bytes().to_vec(),
            is_success: false,
            duration_ms: 100,
        };
        let result = ErrorNormalizer::normalize(&resp);
        assert!(result.is_err());
    }

    #[test]
    fn normalizes_timeout() {
        let result = ErrorNormalizer::timeout_error(Duration::from_secs(30));
        match result {
            AppError::BackendTimeout { .. } => {},
            _ => panic!("Expected BackendTimeout"),
        }
    }

    #[test]
    fn normalizes_connection_failure() {
        let result = ErrorNormalizer::connection_error("Connection refused");
        match result {
            AppError::BackendUnavailable { .. } => {},
            _ => panic!("Expected BackendUnavailable"),
        }
    }
}
```

- [ ] **Step 2: 扩展 AppError 变体**

在 `crates/common/src/error.rs` 的 AppError 枚举中添加：

```rust
#[error("Rate limited")]
RateLimited,

#[error("Circuit breaker open: {0}")]
CircuitBreakerOpen(String),

#[error("Backend timeout after {timeout_ms}ms")]
BackendTimeout { timeout_ms: u64 },

#[error("Backend unavailable: {0}")]
BackendUnavailable(String),

#[error("Backend error: status={status}, detail={detail}")]
BackendError { status: u16, detail: String },
```

对应的 `IntoResponse` 实现也需更新，映射为正确的 HTTP 状态码：
- RateLimited → 429
- CircuitBreakerOpen → 503
- BackendTimeout → 504
- BackendUnavailable → 502
- BackendError → 根据 status 映射（502 兜底）

- [ ] **Step 3: 实现 ErrorNormalizer**

```rust
use crate::types::BackendResponse;
use api_anything_common::error::AppError;
use axum::http::HeaderMap;
use std::time::Duration;

/// 将后端系统的各种错误形式统一转换为 RFC 7807 兼容的 AppError
pub struct ErrorNormalizer;

impl ErrorNormalizer {
    /// 检查后端响应是否表示错误，是则返回规范化的错误
    pub fn normalize(resp: &BackendResponse) -> Result<(), AppError> {
        // HTTP 4xx/5xx 错误
        if resp.status_code >= 400 {
            let body_text = String::from_utf8_lossy(&resp.body);
            return Err(AppError::BackendError {
                status: resp.status_code,
                detail: body_text.to_string(),
            });
        }

        // SOAP Fault 检测（HTTP 200 但 body 包含 fault）
        if !resp.is_success {
            let body_text = String::from_utf8_lossy(&resp.body);
            if body_text.contains("<soap:Fault>") || body_text.contains("<Fault>") {
                // 提取 faultstring
                let detail = Self::extract_soap_fault(&body_text)
                    .unwrap_or_else(|| body_text.to_string());
                return Err(AppError::BackendError {
                    status: 502,
                    detail,
                });
            }
            // 通用非成功响应
            return Err(AppError::BackendError {
                status: 502,
                detail: body_text.to_string(),
            });
        }

        Ok(())
    }

    pub fn timeout_error(timeout: Duration) -> AppError {
        AppError::BackendTimeout {
            timeout_ms: timeout.as_millis() as u64,
        }
    }

    pub fn connection_error(detail: &str) -> AppError {
        AppError::BackendUnavailable(detail.to_string())
    }

    fn extract_soap_fault(body: &str) -> Option<String> {
        // 简单的 faultstring 提取，不依赖 XML 解析器
        let start = body.find("<faultstring>")? + "<faultstring>".len();
        let end = body[start..].find("</faultstring>")? + start;
        Some(body[start..end].to_string())
    }
}
```

- [ ] **Step 4: 更新 gateway lib.rs 添加模块**

- [ ] **Step 5: 运行测试确认通过**

Run: `cargo test --workspace`
Expected: 所有测试通过

- [ ] **Step 6: Commit**

```bash
git commit -am "feat(gateway): add error normalizer with SOAP fault detection"
```

---

### Task 6: 后端调度器 (Backend Dispatcher)

**Files:**
- Create: `crates/gateway/src/dispatcher.rs`

- [ ] **Step 1: 编写调度器测试**

用一个 mock ProtocolAdapter 测试调度流程：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用 mock 适配器，直接返回固定响应
    struct MockAdapter {
        response_body: Vec<u8>,
        should_fail: bool,
    }

    impl ProtocolAdapter for MockAdapter {
        fn transform_request(&self, _req: &GatewayRequest) -> Result<BackendRequest, AppError> {
            Ok(BackendRequest {
                endpoint: "http://mock".to_string(),
                method: Method::GET,
                headers: HeaderMap::new(),
                body: None,
                protocol_params: HashMap::new(),
            })
        }

        async fn execute(&self, _req: &BackendRequest) -> Result<BackendResponse, AppError> {
            if self.should_fail {
                return Err(AppError::BackendUnavailable("Mock failure".into()));
            }
            Ok(BackendResponse {
                status_code: 200,
                headers: HeaderMap::new(),
                body: self.response_body.clone(),
                is_success: true,
                duration_ms: 10,
            })
        }

        fn transform_response(&self, resp: &BackendResponse) -> Result<GatewayResponse, AppError> {
            Ok(GatewayResponse {
                status_code: resp.status_code,
                headers: HashMap::new(),
                body: serde_json::from_slice(&resp.body).unwrap_or(serde_json::Value::Null),
            })
        }

        fn name(&self) -> &str { "mock" }
    }

    #[tokio::test]
    async fn dispatches_request_through_adapter() {
        let adapter = MockAdapter {
            response_body: br#"{"result":"ok"}"#.to_vec(),
            should_fail: false,
        };
        let protection = ProtectionStack::new(1000, 10, 50.0, Duration::from_secs(30), Duration::from_secs(60), 3, Duration::from_secs(30));
        let dispatcher = BackendDispatcher::new(Box::new(adapter), protection);

        let req = make_test_request();
        let resp = dispatcher.dispatch(req).await.unwrap();
        assert_eq!(resp.status_code, 200);
    }

    #[tokio::test]
    async fn respects_rate_limit() {
        let adapter = MockAdapter {
            response_body: br#"{"result":"ok"}"#.to_vec(),
            should_fail: false,
        };
        let protection = ProtectionStack::new(1, 1, 50.0, Duration::from_secs(30), Duration::from_secs(60), 3, Duration::from_secs(30));
        let dispatcher = BackendDispatcher::new(Box::new(adapter), protection);

        let req = make_test_request();
        assert!(dispatcher.dispatch(req.clone()).await.is_ok());
        assert!(dispatcher.dispatch(req).await.is_err()); // rate limited
    }

    #[tokio::test]
    async fn opens_circuit_breaker_on_failures() {
        let adapter = MockAdapter {
            response_body: vec![],
            should_fail: true,
        };
        // 低阈值：1次失败就打开
        let protection = ProtectionStack::new(1000, 100, 50.0, Duration::from_secs(30), Duration::from_secs(60), 3, Duration::from_secs(30));
        let dispatcher = BackendDispatcher::new(Box::new(adapter), protection);

        let req = make_test_request();
        // 多次失败触发熔断
        for _ in 0..5 {
            let _ = dispatcher.dispatch(req.clone()).await;
        }
        // 此时应该被熔断
        let result = dispatcher.dispatch(req).await;
        assert!(result.is_err());
    }

    fn make_test_request() -> GatewayRequest {
        GatewayRequest {
            route_id: Uuid::new_v4(),
            method: Method::GET,
            path: "/test".to_string(),
            headers: HeaderMap::new(),
            query_params: HashMap::new(),
            path_params: HashMap::new(),
            body: None,
            trace_id: "test-trace".to_string(),
        }
    }
}
```

- [ ] **Step 2: 实现 ProtectionStack + BackendDispatcher**

```rust
use crate::adapter::ProtocolAdapter;
use crate::error_normalizer::ErrorNormalizer;
use crate::protection::{CircuitBreaker, ConcurrencySemaphore, RateLimiter};
use crate::types::{GatewayRequest, GatewayResponse};
use api_anything_common::error::AppError;
use std::time::Duration;

/// 保护层栈 — 将限流、信号量、熔断组合为一个整体
pub struct ProtectionStack {
    pub rate_limiter: RateLimiter,
    pub semaphore: ConcurrencySemaphore,
    pub circuit_breaker: CircuitBreaker,
    pub timeout: Duration,
}

impl ProtectionStack {
    pub fn new(
        rps: u32,
        max_concurrent: u32,
        error_threshold: f64,
        window_duration: Duration,
        open_duration: Duration,
        half_open_max: u32,
        timeout: Duration,
    ) -> Self {
        Self {
            rate_limiter: RateLimiter::new(rps, rps), // burst = rps
            semaphore: ConcurrencySemaphore::new(max_concurrent),
            circuit_breaker: CircuitBreaker::new(error_threshold, window_duration, open_duration, half_open_max),
            timeout,
        }
    }
}

/// 后端调度器 — 编排适配器调用，施加保护策略
/// 调用顺序: 限流检查 → 熔断检查 → 获取信号量 → 超时包装执行 → 记录结果 → 错误规范化
pub struct BackendDispatcher {
    adapter: Box<dyn ProtocolAdapter>,
    protection: ProtectionStack,
}

impl BackendDispatcher {
    pub fn new(adapter: Box<dyn ProtocolAdapter>, protection: ProtectionStack) -> Self {
        Self { adapter, protection }
    }

    pub async fn dispatch(&self, req: GatewayRequest) -> Result<GatewayResponse, AppError> {
        // 1. 限流检查
        self.protection.rate_limiter.try_acquire()?;

        // 2. 熔断检查
        if !self.protection.circuit_breaker.allow_request() {
            return Err(AppError::CircuitBreakerOpen(
                format!("Circuit breaker open for adapter '{}'", self.adapter.name())
            ));
        }

        // 3. 获取并发信号量
        let _permit = self.protection.semaphore.acquire().await?;

        // 4. 转换请求
        let backend_req = self.adapter.transform_request(&req)?;

        // 5. 带超时执行后端调用
        let result = tokio::time::timeout(
            self.protection.timeout,
            self.adapter.execute(&backend_req),
        )
        .await;

        let backend_resp = match result {
            Ok(Ok(resp)) => resp,
            Ok(Err(e)) => {
                self.protection.circuit_breaker.record_failure();
                return Err(e);
            }
            Err(_) => {
                self.protection.circuit_breaker.record_failure();
                return Err(ErrorNormalizer::timeout_error(self.protection.timeout));
            }
        };

        // 6. 错误规范化检查
        if let Err(e) = ErrorNormalizer::normalize(&backend_resp) {
            self.protection.circuit_breaker.record_failure();
            return Err(e);
        }

        // 7. 记录成功
        self.protection.circuit_breaker.record_success();

        // 8. 转换响应
        self.adapter.transform_response(&backend_resp)
    }
}
```

- [ ] **Step 3: 更新 gateway lib.rs**

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p api-anything-gateway`
Expected: 所有保护层 + 调度器测试通过

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(gateway): add backend dispatcher with protection stack orchestration"
```

---

### Task 7: 扩展 MetadataRepo — 路由查询方法

**Files:**
- Modify: `crates/metadata/src/repo.rs`
- Modify: `crates/metadata/src/pg.rs`

- [ ] **Step 1: 扩展 MetadataRepo trait**

在 repo.rs 中添加路由相关的查询方法：

```rust
/// 查询所有启用的路由及其关联的后端绑定
async fn list_active_routes_with_bindings(&self) -> Result<Vec<RouteWithBinding>, AppError>;
```

在 `api_anything_common::models` 中定义查询返回类型：

```rust
/// 路由 + 后端绑定的联合查询结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteWithBinding {
    // Route fields
    pub route_id: Uuid,
    pub contract_id: Uuid,
    pub method: HttpMethod,
    pub path: String,
    pub request_schema: Value,
    pub response_schema: Value,
    pub transform_rules: Value,
    pub delivery_guarantee: DeliveryGuarantee,
    // BackendBinding fields
    pub binding_id: Uuid,
    pub protocol: ProtocolType,
    pub endpoint_config: Value,
    pub connection_pool_config: Value,
    pub circuit_breaker_config: Value,
    pub rate_limit_config: Value,
    pub retry_config: Value,
    pub timeout_ms: i64,
    pub auth_mapping: Value,
}
```

- [ ] **Step 2: 实现 PG 查询**

```sql
SELECT
    r.id as route_id, r.contract_id, r.method as "method: HttpMethod",
    r.path, r.request_schema, r.response_schema, r.transform_rules,
    r.delivery_guarantee as "delivery_guarantee: DeliveryGuarantee",
    bb.id as binding_id, bb.protocol as "protocol: ProtocolType",
    bb.endpoint_config, bb.connection_pool_config, bb.circuit_breaker_config,
    bb.rate_limit_config, bb.retry_config, bb.timeout_ms, bb.auth_mapping
FROM routes r
JOIN backend_bindings bb ON r.backend_binding_id = bb.id
WHERE r.enabled = true
```

- [ ] **Step 3: 验证编译**

Run: `DATABASE_URL=postgres://api_anything:api_anything@localhost:5432/api_anything cargo check --workspace`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git commit -am "feat(metadata): add route-with-binding query for gateway router"
```

---

### Task 8: 动态路由器 (Dynamic Router)

**Files:**
- Create: `crates/gateway/src/router.rs`

- [ ] **Step 1: 编写动态路由器测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_exact_path() {
        let mut table = RouteTable::new();
        let route_id = Uuid::new_v4();
        table.insert(Method::GET, "/api/v1/orders", route_id);
        assert_eq!(table.match_route(&Method::GET, "/api/v1/orders"), Some((route_id, HashMap::new())));
    }

    #[test]
    fn matches_path_with_params() {
        let mut table = RouteTable::new();
        let route_id = Uuid::new_v4();
        table.insert(Method::GET, "/api/v1/orders/{id}", route_id);
        let (matched_id, params) = table.match_route(&Method::GET, "/api/v1/orders/abc-123").unwrap();
        assert_eq!(matched_id, route_id);
        assert_eq!(params.get("id").unwrap(), "abc-123");
    }

    #[test]
    fn returns_none_for_unmatched() {
        let table = RouteTable::new();
        assert!(table.match_route(&Method::GET, "/unknown").is_none());
    }

    #[test]
    fn distinguishes_http_methods() {
        let mut table = RouteTable::new();
        let get_id = Uuid::new_v4();
        let post_id = Uuid::new_v4();
        table.insert(Method::GET, "/api/v1/orders", get_id);
        table.insert(Method::POST, "/api/v1/orders", post_id);
        assert_eq!(table.match_route(&Method::GET, "/api/v1/orders").unwrap().0, get_id);
        assert_eq!(table.match_route(&Method::POST, "/api/v1/orders").unwrap().0, post_id);
    }
}
```

- [ ] **Step 2: 实现 RouteTable + DynamicRouter**

```rust
use arc_swap::ArcSwap;
use axum::http::Method;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// 路由表 — 存储 method+path → route_id 的映射
/// 支持 {param} 路径参数
pub struct RouteTable {
    routes: Vec<RouteEntry>,
}

struct RouteEntry {
    method: Method,
    segments: Vec<PathSegment>,
    route_id: Uuid,
}

enum PathSegment {
    Literal(String),
    Param(String),
}

impl RouteTable {
    pub fn new() -> Self {
        Self { routes: Vec::new() }
    }

    pub fn insert(&mut self, method: Method, path: &str, route_id: Uuid) {
        let segments = Self::parse_segments(path);
        self.routes.push(RouteEntry { method, segments, route_id });
    }

    pub fn match_route(&self, method: &Method, path: &str) -> Option<(Uuid, HashMap<String, String>)> {
        let request_segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

        for entry in &self.routes {
            if &entry.method != method { continue; }
            if entry.segments.len() != request_segments.len() { continue; }

            let mut params = HashMap::new();
            let mut matched = true;
            for (seg, req_seg) in entry.segments.iter().zip(&request_segments) {
                match seg {
                    PathSegment::Literal(lit) => {
                        if lit != req_seg { matched = false; break; }
                    }
                    PathSegment::Param(name) => {
                        params.insert(name.clone(), req_seg.to_string());
                    }
                }
            }
            if matched {
                return Some((entry.route_id, params));
            }
        }
        None
    }

    fn parse_segments(path: &str) -> Vec<PathSegment> {
        path.split('/')
            .filter(|s| !s.is_empty())
            .map(|s| {
                if s.starts_with('{') && s.ends_with('}') {
                    PathSegment::Param(s[1..s.len()-1].to_string())
                } else {
                    PathSegment::Literal(s.to_string())
                }
            })
            .collect()
    }
}

/// 动态路由器 — 从元数据加载路由表，通过 ArcSwap 实现 RCU 热更新
pub struct DynamicRouter {
    route_table: ArcSwap<RouteTable>,
}

impl DynamicRouter {
    pub fn new() -> Self {
        Self {
            route_table: ArcSwap::new(Arc::new(RouteTable::new())),
        }
    }

    /// 用新的路由表原子替换当前表
    pub fn update(&self, table: RouteTable) {
        self.route_table.store(Arc::new(table));
    }

    /// 匹配请求路径，返回 route_id 和路径参数
    pub fn match_route(&self, method: &Method, path: &str) -> Option<(Uuid, HashMap<String, String>)> {
        let table = self.route_table.load();
        table.match_route(method, path)
    }
}
```

- [ ] **Step 3: 更新 gateway lib.rs**

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p api-anything-gateway`
Expected: 所有测试通过

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(gateway): add dynamic router with path param matching and RCU hot-reload"
```

---

### Task 9: Gateway 集成 — 挂载到 Platform API

**Files:**
- Modify: `crates/gateway/src/lib.rs` — build_gateway_handler()
- Modify: `crates/platform-api/Cargo.toml` — 添加 gateway 依赖
- Modify: `crates/platform-api/src/lib.rs` — 挂载 gateway 路由
- Modify: `crates/platform-api/src/state.rs` — 添加 DynamicRouter

- [ ] **Step 1: 在 gateway lib.rs 中实现 Axum handler**

创建一个 catch-all Axum handler，接收 `/gw/*` 下的所有请求，通过 DynamicRouter 匹配路由，然后通过 BackendDispatcher 调度：

```rust
use axum::extract::{Path, State, Request};
use axum::response::IntoResponse;
use axum::Json;

/// Gateway 的 catch-all handler
/// 挂载在 /gw/ 路径下，匹配所有子路由
pub async fn gateway_handler(
    State(state): State<GatewayState>,
    req: Request,
) -> Result<impl IntoResponse, AppError> {
    let method = req.method().clone();
    let path = req.uri().path().strip_prefix("/gw").unwrap_or(req.uri().path());

    // 1. 路由匹配
    let (route_id, path_params) = state.router.match_route(&method, path)
        .ok_or_else(|| AppError::NotFound(format!("No route matches {method} {path}")))?;

    // 2. 查找对应的 dispatcher
    let dispatcher = state.dispatchers.get(&route_id)
        .ok_or_else(|| AppError::Internal(format!("No dispatcher for route {route_id}")))?;

    // 3. 构建 GatewayRequest
    let gateway_req = GatewayRequest { /* ... */ };

    // 4. 调度执行
    let resp = dispatcher.dispatch(gateway_req).await?;

    Ok(Json(resp.body))
}
```

- [ ] **Step 2: 定义 GatewayState**

```rust
pub struct GatewayState {
    pub router: Arc<DynamicRouter>,
    pub dispatchers: Arc<DashMap<Uuid, BackendDispatcher>>,
}
```

- [ ] **Step 3: 更新 platform-api 挂载 gateway**

将 `GatewayState` 字段合并到 `AppState`（使用 `axum::extract::FromRef` 提取子 state），然后在 build_app() 中使用 `.fallback()` 或 `.nest()` 挂载 gateway handler。

**State 合并方式：**
```rust
// platform-api/src/state.rs
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub repo: Arc<PgMetadataRepo>,
    pub gateway: GatewayState,  // 新增
}
```

- [ ] **Step 4: 编写集成测试**

创建 `crates/platform-api/tests/gateway_test.rs`，测试：
- `/gw/unknown` 返回 404（无匹配路由）
- 使用 mock adapter 注册一个路由后，`/gw/test` 返回正确响应

- [ ] **Step 5: 运行全量测试**

Run: `DATABASE_URL=postgres://api_anything:api_anything@localhost:5432/api_anything cargo test --workspace`
Expected: 所有测试通过

- [ ] **Step 6: Commit**

```bash
git commit -am "feat: integrate gateway runtime into platform API with dynamic routing"
```

---

## Summary

| Task | 组件 | 产出 |
|------|------|------|
| 1 | Gateway Crate | ProtocolAdapter trait + 请求/响应类型 |
| 2 | Rate Limiter | 令牌桶限流器 + 3 测试 |
| 3 | Circuit Breaker | 滑动窗口熔断器 (3态) + 6 测试 |
| 4 | Semaphore | 并发信号量 + 3 测试 |
| 5 | Error Normalizer | SOAP Fault 检测 + RFC 7807 转换 + 5 测试 |
| 6 | Dispatcher | ProtectionStack + BackendDispatcher + 3 测试 |
| 7 | Metadata 扩展 | 路由+绑定联合查询 |
| 8 | Dynamic Router | 路径匹配 + RCU 热加载 + 4 测试 |
| 9 | 集成 | Gateway 挂载到 Platform API + 集成测试 |

**Phase 1a 验收标准：** gateway crate 编译通过，所有保护层组件单元测试通过，DynamicRouter 可匹配路由，BackendDispatcher 可编排 mock adapter 完成请求-保护-响应全流程，Gateway handler 挂载到 platform-api 并通过集成测试。

**不在 Phase 1a 范围内（后续阶段实现）：**
- 路由表定时轮询刷新（5s 周期）和 Kafka 事件触发更新
- TLS 终结 (rustls)、Auth Guard (JWT)、Request Logger 中间件
- Plugin 动态加载 (libloading .so)
- 连接池 (deadpool) — Phase 1a 的 SOAP adapter 直接使用 reqwest，连接池在 Phase 1b/1c 中添加
