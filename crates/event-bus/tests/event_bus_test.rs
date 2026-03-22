use api_anything_event_bus::{Event, EventBusConfig, EventType, PgEventBus, create_event_bus};
use api_anything_event_bus::bus::EventBus;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

async fn setup() -> PgPool {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://api_anything:api_anything@localhost:5432/api_anything".to_string());
    let pool = PgPool::connect(&database_url).await.expect("Failed to connect to DB");
    // 确保 events 表存在
    sqlx::migrate!("../metadata/src/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");
    pool
}

#[tokio::test]
async fn pg_event_bus_publish_inserts_row() {
    let pool = setup().await;
    let bus = PgEventBus::new(pool.clone());

    let route_id = Uuid::new_v4();
    let event = Event::new(
        EventType::RouteUpdated { route_id },
        json!({"source": "test"}),
    );
    let event_id = event.id;

    bus.publish(event).await.expect("publish should succeed");

    // 验证事件已写入数据库
    let row = sqlx::query_as::<_, (String, serde_json::Value)>(
        "SELECT event_type, payload FROM events WHERE id = $1",
    )
    .bind(event_id)
    .fetch_one(&pool)
    .await
    .expect("event row should exist");

    assert_eq!(row.0, "route_updated");
    assert!(row.1.is_object());
}

#[tokio::test]
async fn pg_event_bus_publish_delivery_failed() {
    let pool = setup().await;
    let bus = PgEventBus::new(pool.clone());

    let record_id = Uuid::new_v4();
    let route_id = Uuid::new_v4();
    let event = Event::new(
        EventType::DeliveryFailed {
            record_id,
            route_id,
            error: "timeout".to_string(),
        },
        json!({}),
    );
    let event_id = event.id;

    bus.publish(event).await.expect("publish should succeed");

    let row = sqlx::query_as::<_, (String,)>(
        "SELECT event_type FROM events WHERE id = $1",
    )
    .bind(event_id)
    .fetch_one(&pool)
    .await
    .expect("event row should exist");

    assert_eq!(row.0, "delivery_failed");
}

#[tokio::test]
async fn event_type_serialization_roundtrip() {
    let route_id = Uuid::new_v4();
    let original = EventType::CircuitBreakerOpened { route_id };

    let json_str = serde_json::to_string(&original).expect("serialize");
    let deserialized: EventType = serde_json::from_str(&json_str).expect("deserialize");

    assert_eq!(original, deserialized);
}

#[tokio::test]
async fn event_type_name_mapping() {
    let id = Uuid::new_v4();
    assert_eq!(EventType::RouteUpdated { route_id: id }.name(), "route_updated");
    assert_eq!(
        EventType::DeliveryFailed { record_id: id, route_id: id, error: "e".into() }.name(),
        "delivery_failed"
    );
    assert_eq!(
        EventType::DeliverySucceeded { record_id: id, route_id: id }.name(),
        "delivery_succeeded"
    );
    assert_eq!(
        EventType::DeadLetter { record_id: id, route_id: id, retry_count: 3 }.name(),
        "dead_letter"
    );
    assert_eq!(
        EventType::GenerationCompleted { project_id: id, contract_id: id, routes_count: 5 }.name(),
        "generation_completed"
    );
    assert_eq!(EventType::CircuitBreakerOpened { route_id: id }.name(), "circuit_breaker_opened");
    assert_eq!(EventType::CircuitBreakerClosed { route_id: id }.name(), "circuit_breaker_closed");
}

#[tokio::test]
async fn factory_defaults_to_pg_event_bus() {
    let pool = setup().await;
    let config = EventBusConfig {
        bus_type: "pg".to_string(),
        kafka_brokers: String::new(),
    };

    // 工厂函数应返回可用的 EventBus 实例
    let bus = create_event_bus(&config, pool.clone());

    let event = Event::new(
        EventType::CircuitBreakerClosed { route_id: Uuid::new_v4() },
        json!({}),
    );

    bus.publish(event).await.expect("factory-created bus should work");
}

#[tokio::test]
async fn factory_unknown_type_defaults_to_pg() {
    let pool = setup().await;
    let config = EventBusConfig {
        bus_type: "unknown".to_string(),
        kafka_brokers: String::new(),
    };

    // 未知 bus_type 也应回退到 PG 实现，不会 panic
    let bus = create_event_bus(&config, pool.clone());

    let event = Event::new(
        EventType::RouteUpdated { route_id: Uuid::new_v4() },
        json!({}),
    );

    bus.publish(event).await.expect("fallback bus should work");
}

#[tokio::test]
async fn event_new_generates_unique_ids() {
    let event1 = Event::new(
        EventType::RouteUpdated { route_id: Uuid::new_v4() },
        json!({}),
    );
    let event2 = Event::new(
        EventType::RouteUpdated { route_id: Uuid::new_v4() },
        json!({}),
    );

    assert_ne!(event1.id, event2.id);
}
