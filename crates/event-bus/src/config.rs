use std::env;

#[derive(Debug, Clone)]
pub struct EventBusConfig {
    /// "pg" 或 "kafka"，决定使用哪种 EventBus 实现
    pub bus_type: String,
    pub kafka_brokers: String,
}

impl EventBusConfig {
    /// 从环境变量构建配置，未设置时默认使用 PG 实现，
    /// 降低本地开发和测试环境的基础设施依赖
    pub fn from_env() -> Self {
        Self {
            bus_type: env::var("EVENT_BUS_TYPE").unwrap_or_else(|_| "pg".to_string()),
            kafka_brokers: env::var("KAFKA_BROKERS")
                .unwrap_or_else(|_| "localhost:9092".to_string()),
        }
    }
}
