use axum::response::IntoResponse;
use axum::Json;

/// GET /api/v1/plugins — 列出所有已加载的插件
/// 当前返回空列表，待 AppState 集成 PluginManager 后替换为实际逻辑
pub async fn list_plugins() -> impl IntoResponse {
    Json(serde_json::json!([]))
}

/// POST /api/v1/plugins/scan — 扫描插件目录并加载新发现的插件
/// 当前返回占位响应，待 PluginManager 集成后才具备真正的扫描能力
pub async fn scan_plugins() -> impl IntoResponse {
    Json(serde_json::json!({"loaded": 0, "message": "Plugin directory not configured"}))
}
