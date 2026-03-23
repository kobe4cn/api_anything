use opentelemetry::metrics::{Counter, Histogram, Meter};
use opentelemetry::KeyValue;

/// 网关核心指标集合
/// 通过 opentelemetry metrics API 暴露，Prometheus 可直接抓取。
/// 各 handler / dispatcher 持有 Arc<GatewayMetrics> 引用，
/// 在请求热路径上调用 record_* 方法记录指标（纳秒级开销）。
pub struct GatewayMetrics {
    /// 网关请求总量，按 route/method/status 维度拆分
    pub request_total: Counter<u64>,
    /// 网关请求端到端延迟分布（含路由匹配 + 后端调用 + 响应序列化）
    pub request_duration: Histogram<f64>,
    /// 后端协议调用延迟分布（仅 adapter.execute 部分）
    pub backend_duration: Histogram<f64>,
    /// 投递重试次数，用于监控重试风暴
    pub retry_total: Counter<u64>,
    /// 死信数量，超过阈值应触发告警
    pub dead_letter_total: Counter<u64>,
}

impl GatewayMetrics {
    pub fn new(meter: &Meter) -> Self {
        Self {
            request_total: meter
                .u64_counter("gateway_request_total")
                .with_description("Total gateway requests")
                .build(),
            request_duration: meter
                .f64_histogram("gateway_request_duration_seconds")
                .with_description("Gateway request latency")
                .build(),
            backend_duration: meter
                .f64_histogram("backend_execute_duration_seconds")
                .with_description("Backend call latency")
                .build(),
            retry_total: meter
                .u64_counter("delivery_retry_total")
                .with_description("Delivery retry count")
                .build(),
            dead_letter_total: meter
                .u64_counter("delivery_dead_letter_total")
                .with_description("Dead letter count")
                .build(),
        }
    }

    /// 记录一次网关请求的指标
    /// duration_secs 来自 Instant::elapsed().as_secs_f64()
    pub fn record_request(&self, route: &str, method: &str, status: u16, duration_secs: f64) {
        let attrs = [
            KeyValue::new("route", route.to_string()),
            KeyValue::new("method", method.to_string()),
            KeyValue::new("status", status.to_string()),
        ];
        self.request_total.add(1, &attrs);
        // duration 只按 route + method 聚合，status 维度在 counter 上已有
        self.request_duration.record(duration_secs, &attrs[..2]);
    }

    /// 记录一次后端协议调用的延迟
    pub fn record_backend(&self, route: &str, protocol: &str, duration_secs: f64) {
        let attrs = [
            KeyValue::new("route", route.to_string()),
            KeyValue::new("protocol", protocol.to_string()),
        ];
        self.backend_duration.record(duration_secs, &attrs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::global;

    fn test_meter() -> Meter {
        // 使用 global noop meter provider，确保测试不依赖外部 collector
        global::meter("test")
    }

    #[test]
    fn new_does_not_panic() {
        let meter = test_meter();
        let _metrics = GatewayMetrics::new(&meter);
    }

    #[test]
    fn record_request_does_not_panic() {
        let meter = test_meter();
        let metrics = GatewayMetrics::new(&meter);
        metrics.record_request("/api/users", "GET", 200, 0.042);
        metrics.record_request("/api/users", "POST", 500, 1.23);
    }

    #[test]
    fn record_backend_does_not_panic() {
        let meter = test_meter();
        let metrics = GatewayMetrics::new(&meter);
        metrics.record_backend("/api/users", "http", 0.015);
        metrics.record_backend("/api/legacy", "soap", 0.350);
    }
}
