/// 保护层全面 E2E 测试 — 直接测试 gateway 组件（RateLimiter、CircuitBreaker、
/// ConcurrencySemaphore、BackendDispatcher），不需要 HTTP 服务器。
/// 覆盖各保护组件的边界行为、状态机转换及在 dispatcher 中的集成表现
use api_anything_gateway::adapter::{BoxFuture, ProtocolAdapter};
use api_anything_gateway::dispatcher::{BackendDispatcher, ProtectionStack};
use api_anything_gateway::protection::circuit_breaker::CircuitState;
use api_anything_gateway::protection::{CircuitBreaker, ConcurrencySemaphore, RateLimiter};
use api_anything_gateway::types::*;
use api_anything_common::error::AppError;
use axum::http::{HeaderMap, Method};
use std::collections::HashMap;
use std::time::Duration;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Rate Limiter 深度测试
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rate_limiter_allows_burst_then_rejects() {
    let limiter = RateLimiter::new(5, 5);
    for i in 0..5 {
        assert!(
            limiter.try_acquire().is_ok(),
            "Request {} within burst should pass",
            i
        );
    }
    // 令牌桶已耗尽，第 6 个请求应被拒绝
    assert!(
        limiter.try_acquire().is_err(),
        "6th request should be rejected"
    );
}

#[tokio::test]
async fn rate_limiter_recovers_after_wait() {
    // 100 rps、burst=1：耗尽 1 个令牌后等待 ~20ms 应补充 >=1 个令牌
    let limiter = RateLimiter::new(100, 1);
    assert!(limiter.try_acquire().is_ok());
    assert!(limiter.try_acquire().is_err());
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        limiter.try_acquire().is_ok(),
        "Token should be refilled after waiting"
    );
}

#[tokio::test]
async fn rate_limiter_independent_per_instance() {
    // 不同实例维护独立的令牌桶，互不影响
    let limiter1 = RateLimiter::new(1, 1);
    let limiter2 = RateLimiter::new(1, 1);
    assert!(limiter1.try_acquire().is_ok());
    assert!(limiter2.try_acquire().is_ok());
    assert!(limiter1.try_acquire().is_err());
    assert!(limiter2.try_acquire().is_err());
}

#[tokio::test]
async fn rate_limiter_large_burst_allows_spike() {
    // burst 远大于 rps，验证短时间大量请求被允许通过
    let limiter = RateLimiter::new(1, 20);
    for _ in 0..20 {
        assert!(limiter.try_acquire().is_ok());
    }
    assert!(limiter.try_acquire().is_err());
}

// ---------------------------------------------------------------------------
// Circuit Breaker 深度测试
// ---------------------------------------------------------------------------

#[test]
fn circuit_breaker_full_state_machine() {
    // open_duration = 100ms, half_open_max = 2
    let cb = CircuitBreaker::new(50.0, Duration::from_secs(30), Duration::from_millis(100), 2);

    // Closed: 混合成功失败，低于阈值 — 需要超过 min_volume (half_open_max=2) 才评估
    for _ in 0..3 {
        cb.record_success();
    }
    for _ in 0..2 {
        cb.record_failure();
    }
    // 2/(3+2) = 40% < 50% → 仍然关闭
    assert_eq!(cb.state(), CircuitState::Closed);

    // Closed → Open: 连续失败使错误率超过 50%
    for _ in 0..10 {
        cb.record_failure();
    }
    assert_eq!(cb.state(), CircuitState::Open);
    assert!(!cb.allow_request());

    // Open → HalfOpen: 等待 open_duration
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(cb.state(), CircuitState::HalfOpen);
    assert!(cb.allow_request());

    // HalfOpen → Closed: 连续成功达到 half_open_max_requests
    cb.record_success();
    cb.record_success();
    assert_eq!(cb.state(), CircuitState::Closed);
}

#[test]
fn circuit_breaker_half_open_reopens_on_failure() {
    // HalfOpen 阶段出现一次失败就应立即重新打开
    let cb = CircuitBreaker::new(50.0, Duration::from_secs(30), Duration::from_millis(50), 3);
    for _ in 0..10 {
        cb.record_failure();
    }
    std::thread::sleep(Duration::from_millis(60));
    assert_eq!(cb.state(), CircuitState::HalfOpen);

    cb.record_failure();
    assert_eq!(cb.state(), CircuitState::Open);
}

#[test]
fn circuit_breaker_window_reset() {
    // 窗口过期后重置计数，新窗口从零开始统计
    let cb = CircuitBreaker::new(50.0, Duration::from_millis(50), Duration::from_secs(60), 3);
    // 在短窗口内制造一些失败（但不足以超过 min_volume 触发打开）
    for _ in 0..3 {
        cb.record_failure();
    }
    // 等待窗口过期
    std::thread::sleep(Duration::from_millis(60));
    // 新窗口内记录成功
    cb.record_success();
    assert_eq!(
        cb.state(),
        CircuitState::Closed,
        "Window should have reset, circuit breaker should stay closed"
    );
}

#[test]
fn circuit_breaker_stays_closed_below_threshold() {
    let cb = CircuitBreaker::new(50.0, Duration::from_secs(30), Duration::from_secs(60), 3);
    // 错误率 30% (3/10) < 50%
    for _ in 0..7 {
        cb.record_success();
    }
    for _ in 0..3 {
        cb.record_failure();
    }
    assert_eq!(cb.state(), CircuitState::Closed);
    assert!(cb.allow_request());
}

// ---------------------------------------------------------------------------
// Semaphore 深度测试
// ---------------------------------------------------------------------------

#[tokio::test]
async fn semaphore_limits_concurrency() {
    let sem = ConcurrencySemaphore::new(2);
    let p1 = sem.acquire().await.unwrap();
    let p2 = sem.acquire().await.unwrap();
    assert_eq!(sem.available_permits(), 0);
    assert!(sem.try_acquire().is_err());

    // 释放一个许可后应可再次获取
    drop(p1);
    assert_eq!(sem.available_permits(), 1);
    let _p3 = sem.acquire().await.unwrap();
    assert_eq!(sem.available_permits(), 0);

    // 全部释放
    drop(p2);
    drop(_p3);
    assert_eq!(sem.available_permits(), 2);
}

#[tokio::test]
async fn semaphore_try_acquire_returns_err_when_exhausted() {
    let sem = ConcurrencySemaphore::new(1);
    let _permit = sem.acquire().await.unwrap();
    assert!(
        sem.try_acquire().is_err(),
        "try_acquire should fail when all permits are taken"
    );
}

#[tokio::test]
async fn semaphore_releases_on_drop() {
    let sem = ConcurrencySemaphore::new(1);
    {
        let _permit = sem.acquire().await.unwrap();
        assert_eq!(sem.available_permits(), 0);
    }
    // RAII 释放后许可归还
    assert_eq!(sem.available_permits(), 1);
}

// ---------------------------------------------------------------------------
// 可配置 Mock Adapter — 用于 BackendDispatcher 集成测试
// ---------------------------------------------------------------------------

struct ConfigurableMockAdapter {
    should_fail: bool,
    delay_ms: u64,
    response: Vec<u8>,
}

impl ProtocolAdapter for ConfigurableMockAdapter {
    fn transform_request(&self, _req: &GatewayRequest) -> Result<BackendRequest, AppError> {
        Ok(BackendRequest {
            endpoint: "http://mock".to_string(),
            method: Method::GET,
            headers: HeaderMap::new(),
            body: None,
            protocol_params: HashMap::new(),
        })
    }

    fn execute<'a>(
        &'a self,
        _req: &'a BackendRequest,
    ) -> BoxFuture<'a, Result<BackendResponse, AppError>> {
        Box::pin(async move {
            if self.delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            }
            if self.should_fail {
                return Err(AppError::BackendUnavailable("Mock failure".into()));
            }
            Ok(BackendResponse {
                status_code: 200,
                headers: HeaderMap::new(),
                body: self.response.clone(),
                is_success: true,
                duration_ms: self.delay_ms,
            })
        })
    }

    fn transform_response(
        &self,
        resp: &BackendResponse,
    ) -> Result<GatewayResponse, AppError> {
        Ok(GatewayResponse {
            status_code: resp.status_code,
            headers: HashMap::new(),
            body: serde_json::from_slice(&resp.body).unwrap_or(serde_json::Value::Null),
        })
    }

    fn name(&self) -> &str {
        "configurable-mock"
    }
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

// ---------------------------------------------------------------------------
// BackendDispatcher 集成测试
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dispatcher_applies_rate_limit() {
    let adapter = ConfigurableMockAdapter {
        should_fail: false,
        delay_ms: 0,
        response: br#"{"ok":true}"#.to_vec(),
    };
    // rps=2, burst=2 → 令牌桶初始有 2 个令牌
    let protection = ProtectionStack::new(
        2,    // rps
        100,  // max_concurrent
        50.0, // error_threshold
        Duration::from_secs(30),
        Duration::from_secs(60),
        3,
        Duration::from_secs(30),
    );
    let dispatcher = BackendDispatcher::new(Box::new(adapter), protection);

    assert!(dispatcher.dispatch(make_test_request()).await.is_ok());
    assert!(dispatcher.dispatch(make_test_request()).await.is_ok());
    // 第 3 次应触发限流
    let result = dispatcher.dispatch(make_test_request()).await;
    assert!(
        matches!(result, Err(AppError::RateLimited)),
        "3rd request should be rate limited, got: {:?}",
        result
    );
}

#[tokio::test]
async fn dispatcher_applies_circuit_breaker() {
    let adapter = ConfigurableMockAdapter {
        should_fail: true,
        delay_ms: 0,
        response: vec![],
    };
    let protection = ProtectionStack::new(
        1000, // 高 rps 避免限流干扰
        100,
        50.0,
        Duration::from_secs(30),
        Duration::from_secs(60),
        3,    // min_volume 也是 3
        Duration::from_secs(30),
    );
    let dispatcher = BackendDispatcher::new(Box::new(adapter), protection);

    // 持续失败直到熔断器打开（需超过 min_volume=3 且错误率 >=50%）
    for _ in 0..10 {
        let _ = dispatcher.dispatch(make_test_request()).await;
    }
    let result = dispatcher.dispatch(make_test_request()).await;
    assert!(
        matches!(result, Err(AppError::CircuitBreakerOpen(_))),
        "Should get CircuitBreakerOpen after many failures, got: {:?}",
        result
    );
}

#[tokio::test]
async fn dispatcher_applies_timeout() {
    let adapter = ConfigurableMockAdapter {
        should_fail: false,
        delay_ms: 500, // 远大于 timeout
        response: br#"{"ok":true}"#.to_vec(),
    };
    let protection = ProtectionStack::new(
        1000,
        100,
        50.0,
        Duration::from_secs(30),
        Duration::from_secs(60),
        3,
        Duration::from_millis(50), // 50ms 超时
    );
    let dispatcher = BackendDispatcher::new(Box::new(adapter), protection);

    let result = dispatcher.dispatch(make_test_request()).await;
    assert!(
        matches!(result, Err(AppError::BackendTimeout { .. })),
        "Should get BackendTimeout when adapter exceeds timeout, got: {:?}",
        result
    );
}

#[tokio::test]
async fn dispatcher_records_success_in_circuit_breaker() {
    let adapter = ConfigurableMockAdapter {
        should_fail: false,
        delay_ms: 0,
        response: br#"{"ok":true}"#.to_vec(),
    };
    let protection = ProtectionStack::new(
        1000,
        100,
        50.0,
        Duration::from_secs(30),
        Duration::from_secs(60),
        3,
        Duration::from_secs(30),
    );
    // 成功请求应使熔断器保持关闭状态
    let cb_state_before = protection.circuit_breaker.state();
    assert_eq!(cb_state_before, CircuitState::Closed);

    let dispatcher = BackendDispatcher::new(Box::new(adapter), protection);
    for _ in 0..5 {
        assert!(dispatcher.dispatch(make_test_request()).await.is_ok());
    }
    // 由于 ProtectionStack 被 move 到 dispatcher 里了，无法直接访问，
    // 但可以通过继续发请求验证 dispatcher 仍然正常工作（未被熔断）
    assert!(
        dispatcher.dispatch(make_test_request()).await.is_ok(),
        "Circuit breaker should remain closed after successful requests"
    );
}
