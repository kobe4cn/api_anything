use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use crate::state::AppState;
use std::time::Duration;
use uuid::Uuid;

/// WebSocket 升级入口。客户端连接 /ws 后升级为 WebSocket，
/// 服务端以 2 秒间隔轮询 events 表并推送新事件。
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_ws(socket, state))
}

/// WebSocket 长连接处理逻辑。
/// 采用轮询 events 表而非内存广播，原因是多副本部署时所有实例共享同一数据库，
/// 无需额外的跨进程消息通道即可保证事件不丢失。
async fn handle_ws(mut socket: WebSocket, state: AppState) {
    let mut interval = tokio::time::interval(Duration::from_secs(2));
    let mut last_event_id: Option<Uuid> = None;

    loop {
        interval.tick().await;

        // $1::uuid IS NULL 处理首次连接时 last_event_id 为 None 的情况，
        // 此时返回最近 10 条事件作为初始快照
        let result = sqlx::query_as::<_, (Uuid, String, serde_json::Value, chrono::DateTime<chrono::Utc>)>(
            "SELECT id, event_type, payload, created_at FROM events WHERE ($1::uuid IS NULL OR id > $1) ORDER BY created_at ASC LIMIT 10"
        )
        .bind(last_event_id)
        .fetch_all(&state.db)
        .await;

        match result {
            Ok(events) => {
                for (id, event_type, payload, created_at) in &events {
                    let msg = serde_json::json!({
                        "id": id,
                        "type": event_type,
                        "payload": payload,
                        "timestamp": created_at,
                    });
                    if socket
                        .send(Message::Text(msg.to_string().into()))
                        .await
                        .is_err()
                    {
                        return; // 客户端已断开，退出循环释放资源
                    }
                    last_event_id = Some(*id);
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "WebSocket event query failed");
            }
        }
    }
}
