use crate::event_types::Event;
use std::future::Future;
use std::pin::Pin;

/// BoxFuture 消除 async trait 方法的 dyn 兼容性问题，
/// 使 EventBus 可以作为 trait object（Box<dyn EventBus>）在运行时多态分发
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// 事件总线抽象：所有实现（PG / Kafka）通过此 trait 对上层透明可替换
pub trait EventBus: Send + Sync {
    fn publish<'a>(&'a self, event: Event) -> BoxFuture<'a, Result<(), anyhow::Error>>;
}

/// 事件处理器抽象，subscribe 侧使用
pub trait EventHandler: Send + Sync {
    fn handle<'a>(&'a self, event: Event) -> BoxFuture<'a, Result<(), anyhow::Error>>;
}
