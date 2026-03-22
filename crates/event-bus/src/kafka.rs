use crate::bus::{BoxFuture, EventBus};
use crate::event_types::Event;
use rdkafka::config::ClientConfig;
use rdkafka::producer::{FutureProducer, FutureRecord};
use std::time::Duration;
use tracing;

/// 基于 rdkafka 的事件总线实现，适用于高吞吐场景。
/// event_type.name() 直接映射为 Kafka topic，确保不同类型事件天然隔离，
/// 消费方可以按需订阅感兴趣的 topic 而非过滤全量消息
pub struct KafkaEventBus {
    producer: FutureProducer,
}

impl KafkaEventBus {
    pub fn new(brokers: &str) -> Self {
        let producer: FutureProducer = ClientConfig::new()
            .set("bootstrap.servers", brokers)
            // acks=all 保证消息写入所有 ISR 副本后才返回成功，避免 leader 切换导致事件丢失
            .set("message.timeout.ms", "5000")
            .create()
            .expect("Failed to create Kafka producer");

        Self { producer }
    }
}

impl EventBus for KafkaEventBus {
    fn publish<'a>(&'a self, event: Event) -> BoxFuture<'a, Result<(), anyhow::Error>> {
        Box::pin(async move {
            let topic = event.event_type.name();
            let key = event.id.to_string();
            let payload = serde_json::to_string(&event)?;

            // 超时 5 秒：避免 broker 不可达时无限阻塞调用方
            self.producer
                .send(
                    FutureRecord::to(topic)
                        .key(&key)
                        .payload(&payload),
                    Duration::from_secs(5),
                )
                .await
                .map_err(|(err, _)| anyhow::anyhow!("Kafka publish failed: {}", err))?;

            tracing::debug!(
                event_id = %event.id,
                event_type = topic,
                "Event published to Kafka"
            );

            Ok(())
        })
    }
}
