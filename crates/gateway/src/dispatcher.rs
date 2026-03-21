use crate::adapter::{BoxFuture, ProtocolAdapter};
use crate::error_normalizer::ErrorNormalizer;
use crate::protection::{CircuitBreaker, ConcurrencySemaphore, RateLimiter};
use crate::types::{GatewayRequest, GatewayResponse};
use api_anything_common::error::AppError;
use std::time::Duration;

/// 将所有保护机制组合到一个统一的配置结构中，
/// 便于在路由级别按需设置差异化的流控参数
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
            rate_limiter: RateLimiter::new(rps, rps),
            semaphore: ConcurrencySemaphore::new(max_concurrent),
            circuit_breaker: CircuitBreaker::new(
                error_threshold,
                window_duration,
                open_duration,
                half_open_max,
            ),
            timeout,
        }
    }
}

/// 按 rate limit → circuit breaker → semaphore → timeout → execute → normalize 顺序调度，
/// 确保廉价的拒绝操作（限流、熔断）先于昂贵的资源占用（信号量、网络 IO）执行
pub struct BackendDispatcher {
    adapter: Box<dyn ProtocolAdapter>,
    protection: ProtectionStack,
}

impl BackendDispatcher {
    pub fn new(adapter: Box<dyn ProtocolAdapter>, protection: ProtectionStack) -> Self {
        Self { adapter, protection }
    }

    pub async fn dispatch(&self, req: GatewayRequest) -> Result<GatewayResponse, AppError> {
        // 1. 限流：令牌桶校验，无令牌时立即拒绝，不占用线程等待
        self.protection.rate_limiter.try_acquire()?;

        // 2. 熔断器检查：Open 状态直接返回，避免向已故障的后端继续施压
        if !self.protection.circuit_breaker.allow_request() {
            return Err(AppError::CircuitBreakerOpen(
                format!("Circuit breaker open for '{}'", self.adapter.name()),
            ));
        }

        // 3. 并发限制：获取信号量许可，permit 通过 RAII 自动归还
        let _permit = self.protection.semaphore.acquire().await?;

        // 4. 协议适配：将网关请求转换为后端协议请求
        let backend_req = self.adapter.transform_request(&req)?;

        // 5. 带超时执行：区分超时错误与后端返回的业务错误，分别记录失败
        let result = tokio::time::timeout(
            self.protection.timeout,
            self.adapter.execute(&backend_req),
        )
        .await;

        let backend_resp = match result {
            Ok(Ok(resp)) => resp,
            Ok(Err(e)) => {
                // 后端返回错误（连接失败、协议错误等），计入熔断器失败窗口
                self.protection.circuit_breaker.record_failure();
                return Err(e);
            }
            Err(_) => {
                // tokio::time::timeout 超时，计入熔断器失败窗口
                self.protection.circuit_breaker.record_failure();
                return Err(ErrorNormalizer::timeout_error(self.protection.timeout));
            }
        };

        // 6. 错误规范化：将 HTTP 4xx/5xx 和 SOAP Fault 统一转为 AppError
        if let Err(e) = ErrorNormalizer::normalize(&backend_resp) {
            self.protection.circuit_breaker.record_failure();
            return Err(e);
        }

        // 7. 成功路径：通知熔断器当前窗口有一次成功请求
        self.protection.circuit_breaker.record_success();

        // 8. 响应转换：将后端原始响应映射为统一的 GatewayResponse
        self.adapter.transform_response(&backend_resp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use axum::http::{HeaderMap, Method};
    use std::collections::HashMap;
    use uuid::Uuid;

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

        fn execute<'a>(&'a self, _req: &'a BackendRequest) -> BoxFuture<'a, Result<BackendResponse, AppError>> {
            Box::pin(async move {
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
            "mock"
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

    #[tokio::test]
    async fn dispatches_through_adapter() {
        let adapter = MockAdapter {
            response_body: br#"{"result":"ok"}"#.to_vec(),
            should_fail: false,
        };
        let protection = ProtectionStack::new(
            1000,
            10,
            50.0,
            Duration::from_secs(30),
            Duration::from_secs(60),
            3,
            Duration::from_secs(30),
        );
        let dispatcher = BackendDispatcher::new(Box::new(adapter), protection);
        let resp = dispatcher.dispatch(make_test_request()).await.unwrap();
        assert_eq!(resp.status_code, 200);
    }

    #[tokio::test]
    async fn respects_rate_limit() {
        let adapter = MockAdapter {
            response_body: br#"{"ok":true}"#.to_vec(),
            should_fail: false,
        };
        let protection = ProtectionStack::new(
            1,
            1,
            50.0,
            Duration::from_secs(30),
            Duration::from_secs(60),
            3,
            Duration::from_secs(30),
        );
        let dispatcher = BackendDispatcher::new(Box::new(adapter), protection);
        // 令牌桶初始只有 1 个令牌，第二次请求必然触发限流
        assert!(dispatcher.dispatch(make_test_request()).await.is_ok());
        assert!(dispatcher.dispatch(make_test_request()).await.is_err());
    }

    #[tokio::test]
    async fn records_failures_in_circuit_breaker() {
        let adapter = MockAdapter {
            response_body: vec![],
            should_fail: true,
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
        let dispatcher = BackendDispatcher::new(Box::new(adapter), protection);
        // 持续失败直到错误率超过 50% 阈值（最小样本量 3），熔断器打开
        for _ in 0..10 {
            let _ = dispatcher.dispatch(make_test_request()).await;
        }
        let result = dispatcher.dispatch(make_test_request()).await;
        assert!(matches!(result, Err(AppError::CircuitBreakerOpen(_))));
    }
}
