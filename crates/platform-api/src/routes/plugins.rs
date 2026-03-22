use api_anything_plugin_sdk::PluginInfo;
use axum::{extract::State, routing::get, Json, Router};
use std::sync::Arc;

use api_anything_gateway::plugin_loader::PluginManager;

/// 插件管理 API 路由
/// 集成步骤在后续任务中完成，此处仅定义路由结构
pub fn router() -> Router<Arc<PluginManager>> {
    Router::new()
        .route("/api/v1/plugins", get(list_plugins))
        .route("/api/v1/plugins/scan", axum::routing::post(scan_plugins))
}

/// GET /api/v1/plugins — 列出所有已加载的插件
async fn list_plugins(
    State(manager): State<Arc<PluginManager>>,
) -> Json<Vec<PluginInfo>> {
    Json(manager.list_plugins())
}

/// POST /api/v1/plugins/scan — 扫描插件目录并加载新发现的插件
async fn scan_plugins(
    State(manager): State<Arc<PluginManager>>,
) -> Result<Json<Vec<PluginInfo>>, String> {
    manager
        .scan_and_load()
        .map(Json)
        .map_err(|e| format!("Scan failed: {e}"))
}
