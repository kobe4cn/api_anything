use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// 业务事件枚举，覆盖路由、投递、熔断等核心生命周期节点。
/// 新增业务事件只需扩展此枚举，所有 EventBus 实现无需修改。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum EventType {
    RouteUpdated {
        route_id: Uuid,
    },
    DeliveryFailed {
        record_id: Uuid,
        route_id: Uuid,
        error: String,
    },
    DeliverySucceeded {
        record_id: Uuid,
        route_id: Uuid,
    },
    DeadLetter {
        record_id: Uuid,
        route_id: Uuid,
        retry_count: u32,
    },
    GenerationCompleted {
        project_id: Uuid,
        contract_id: Uuid,
        routes_count: usize,
    },
    CircuitBreakerOpened {
        route_id: Uuid,
    },
    CircuitBreakerClosed {
        route_id: Uuid,
    },
}

impl EventType {
    /// 返回事件类型标识字符串，Kafka 实现用作 topic 名称，PG 实现用作查询过滤条件
    pub fn name(&self) -> &'static str {
        match self {
            EventType::RouteUpdated { .. } => "route_updated",
            EventType::DeliveryFailed { .. } => "delivery_failed",
            EventType::DeliverySucceeded { .. } => "delivery_succeeded",
            EventType::DeadLetter { .. } => "dead_letter",
            EventType::GenerationCompleted { .. } => "generation_completed",
            EventType::CircuitBreakerOpened { .. } => "circuit_breaker_opened",
            EventType::CircuitBreakerClosed { .. } => "circuit_breaker_closed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: Uuid,
    pub event_type: EventType,
    pub timestamp: DateTime<Utc>,
    /// 附加上下文数据，各消费方按需解读，避免在 EventType 枚举中穷举所有业务字段
    pub payload: Value,
}

impl Event {
    /// 便捷构造方法，自动生成 id 和 timestamp，调用方只需关心事件类型和负载
    pub fn new(event_type: EventType, payload: Value) -> Self {
        Self {
            id: Uuid::new_v4(),
            event_type,
            timestamp: Utc::now(),
            payload,
        }
    }
}
