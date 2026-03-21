use std::sync::Mutex;
use std::time::{Duration, Instant};
use api_anything_common::error::AppError;

/// 令牌桶限流器
pub struct RateLimiter {
    inner: Mutex<TokenBucket>,
}

struct TokenBucket {
    tokens: f64,
    max_tokens: f64,
    // 每秒补充的令牌数，控制稳态请求速率
    refill_rate: f64,
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
        // 按经过时间线性补充令牌，上限为桶容量，防止长时间空闲后突发流量绕过限流
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.max_tokens);
        self.last_refill = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn allows_requests_within_limit() {
        let limiter = RateLimiter::new(10, 10);
        for _ in 0..10 {
            assert!(limiter.try_acquire().is_ok());
        }
    }

    #[tokio::test]
    async fn rejects_requests_over_limit() {
        let limiter = RateLimiter::new(2, 2);
        assert!(limiter.try_acquire().is_ok());
        assert!(limiter.try_acquire().is_ok());
        assert!(limiter.try_acquire().is_err());
    }

    #[tokio::test]
    async fn refills_tokens_over_time() {
        let limiter = RateLimiter::new(100, 1);
        assert!(limiter.try_acquire().is_ok());
        assert!(limiter.try_acquire().is_err());
        // 等待 15ms 后，100 req/s 的补充速率应已积累超过 1 个令牌
        tokio::time::sleep(Duration::from_millis(15)).await;
        assert!(limiter.try_acquire().is_ok());
    }
}
