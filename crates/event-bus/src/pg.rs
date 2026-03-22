use crate::bus::{BoxFuture, EventBus};
use crate::event_types::Event;
use sqlx::PgPool;
use tracing;

/// 基于 PostgreSQL events 表的事件总线实现。
/// 选择数据库轮询而非 NOTIFY/LISTEN 是因为：
/// 1. events 表天然支持持久化和故障恢复，NOTIFY 消息是瞬态的
/// 2. 多实例部署时 LISTEN 需要每个实例维护连接，轮询可以通过 SELECT FOR UPDATE SKIP LOCKED 分发
pub struct PgEventBus {
    pool: PgPool,
}

impl PgEventBus {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

impl EventBus for PgEventBus {
    fn publish<'a>(&'a self, event: Event) -> BoxFuture<'a, Result<(), anyhow::Error>> {
        Box::pin(async move {
            let event_type_name = event.event_type.name();
            // 将整个 event 序列化为 payload JSON，保留完整事件信息供消费方还原
            let payload = serde_json::to_value(&event)?;

            sqlx::query(
                r#"
                INSERT INTO events (id, event_type, payload, created_at)
                VALUES ($1, $2, $3, $4)
                "#,
            )
            .bind(event.id)
            .bind(event_type_name)
            .bind(&payload)
            .bind(event.timestamp)
            .execute(&self.pool)
            .await?;

            tracing::debug!(
                event_id = %event.id,
                event_type = event_type_name,
                "Event published to PostgreSQL"
            );

            Ok(())
        })
    }
}
