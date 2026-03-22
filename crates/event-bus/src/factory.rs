use crate::bus::EventBus;
use crate::config::EventBusConfig;
use crate::pg::PgEventBus;
use sqlx::PgPool;

/// 根据配置创建对应的 EventBus 实例。
/// Kafka 实现需要编译时启用 `kafka` feature，运行时指定 EVENT_BUS_TYPE=kafka 但未启用 feature 会 panic，
/// 通过编译时和运行时双重检查防止配置与构建不匹配的隐蔽错误
pub fn create_event_bus(config: &EventBusConfig, pool: PgPool) -> Box<dyn EventBus> {
    match config.bus_type.as_str() {
        #[cfg(feature = "kafka")]
        "kafka" => {
            tracing::info!(brokers = %config.kafka_brokers, "Using Kafka event bus");
            Box::new(crate::kafka::KafkaEventBus::new(&config.kafka_brokers))
        }
        #[cfg(not(feature = "kafka"))]
        "kafka" => {
            panic!(
                "EVENT_BUS_TYPE=kafka but the 'kafka' feature is not enabled. \
                 Rebuild with `--features kafka` to use Kafka event bus."
            );
        }
        _ => {
            tracing::info!("Using PostgreSQL event bus");
            Box::new(PgEventBus::new(pool))
        }
    }
}
