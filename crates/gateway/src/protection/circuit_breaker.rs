use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

pub struct CircuitBreaker {
    inner: Mutex<CircuitBreakerInner>,
}

struct CircuitBreakerInner {
    error_threshold_percent: f64,
    window_duration: Duration,
    // Open 状态保持多久后允许进入 HalfOpen 探测
    open_duration: Duration,
    // HalfOpen 阶段需要连续成功多少次才认为服务已恢复
    half_open_max_requests: u32,
    successes: u32,
    failures: u32,
    window_start: Instant,
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

    /// 返回当前熔断器状态，同时处理 Open→HalfOpen 的超时转换
    pub fn state(&self) -> CircuitState {
        let mut inner = self.inner.lock().unwrap();
        inner.check_open_timeout();
        inner.state
    }

    /// Closed 或 HalfOpen 时允许请求通过，Open 时拒绝
    pub fn allow_request(&self) -> bool {
        let mut inner = self.inner.lock().unwrap();
        inner.check_open_timeout();
        matches!(inner.state, CircuitState::Closed | CircuitState::HalfOpen)
    }

    pub fn record_success(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.maybe_reset_window();
        match inner.state {
            CircuitState::Closed => {
                inner.successes += 1;
            }
            CircuitState::HalfOpen => {
                inner.half_open_successes += 1;
                // 达到探测阈值后认为服务已恢复，关闭熔断器并重置所有计数
                if inner.half_open_successes >= inner.half_open_max_requests {
                    inner.transition_to_closed();
                }
            }
            CircuitState::Open => {}
        }
    }

    pub fn record_failure(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.maybe_reset_window();
        match inner.state {
            CircuitState::Closed => {
                inner.failures += 1;
                // 要求最小样本量超过 half_open_max_requests 才评估错误率，
                // 防止冷启动阶段极少量请求就触发熔断（小样本误差大）
                let total = inner.successes + inner.failures;
                let min_volume = inner.half_open_max_requests;
                if total > min_volume {
                    let error_rate = (inner.failures as f64 / total as f64) * 100.0;
                    if error_rate >= inner.error_threshold_percent {
                        inner.transition_to_open();
                    }
                }
            }
            // HalfOpen 期间出现失败说明服务尚未完全恢复，立即重新打开熔断器
            CircuitState::HalfOpen => {
                inner.transition_to_open();
            }
            CircuitState::Open => {}
        }
    }
}

impl CircuitBreakerInner {
    /// 检查 Open 状态是否已超过等待时长，超过则切换到 HalfOpen 开始探测
    fn check_open_timeout(&mut self) {
        if self.state == CircuitState::Open {
            if let Some(opened_at) = self.opened_at {
                if opened_at.elapsed() >= self.open_duration {
                    self.state = CircuitState::HalfOpen;
                    self.half_open_successes = 0;
                }
            }
        }
    }

    /// 滑动窗口：若当前窗口已过期则重置计数，保证错误率基于最近时段
    fn maybe_reset_window(&mut self) {
        if self.window_start.elapsed() >= self.window_duration {
            self.successes = 0;
            self.failures = 0;
            self.window_start = Instant::now();
        }
    }

    fn transition_to_open(&mut self) {
        tracing::warn!(
            failures = self.failures,
            successes = self.successes,
            "Circuit breaker opened due to high error rate"
        );
        self.state = CircuitState::Open;
        self.opened_at = Some(Instant::now());
    }

    fn transition_to_closed(&mut self) {
        self.state = CircuitState::Closed;
        self.successes = 0;
        self.failures = 0;
        self.opened_at = None;
        self.half_open_successes = 0;
        self.window_start = Instant::now();
    }
}

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
        for _ in 0..10 {
            cb.record_failure();
        }
        assert_eq!(cb.state(), CircuitState::Open);
    }

    #[test]
    fn stays_closed_when_error_rate_below_threshold() {
        let cb = CircuitBreaker::new(50.0, Duration::from_secs(30), Duration::from_secs(60), 3);
        for _ in 0..3 {
            cb.record_failure();
        }
        for _ in 0..7 {
            cb.record_success();
        }
        assert_eq!(cb.state(), CircuitState::Closed);
    }

    #[test]
    fn allows_check_returns_false_when_open() {
        let cb = CircuitBreaker::new(50.0, Duration::from_secs(30), Duration::from_secs(60), 3);
        for _ in 0..10 {
            cb.record_failure();
        }
        assert!(!cb.allow_request());
    }

    #[test]
    fn transitions_to_half_open_after_timeout() {
        let cb =
            CircuitBreaker::new(50.0, Duration::from_secs(30), Duration::from_millis(50), 3);
        for _ in 0..10 {
            cb.record_failure();
        }
        assert_eq!(cb.state(), CircuitState::Open);
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(cb.state(), CircuitState::HalfOpen);
        assert!(cb.allow_request());
    }

    #[test]
    fn half_open_closes_on_enough_successes() {
        let cb =
            CircuitBreaker::new(50.0, Duration::from_secs(30), Duration::from_millis(50), 1);
        for _ in 0..10 {
            cb.record_failure();
        }
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(cb.state(), CircuitState::HalfOpen);
        cb.record_success();
        assert_eq!(cb.state(), CircuitState::Closed);
    }
}
