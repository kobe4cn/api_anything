use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use api_anything_common::error::AppError;
use api_anything_gateway::types::GatewayRequest;
use api_anything_metadata::MetadataRepo;
use api_anything_sandbox::mock_layer::MockLayer;
use api_anything_sandbox::replay_layer::ReplayLayer;
use api_anything_sandbox::proxy_layer::ProxyLayer;
use crate::state::AppState;
use std::collections::HashMap;
use uuid::Uuid;

/// 沙箱通配 handler — 接收所有 /sandbox/* 请求，根据 X-Sandbox-Mode 头分派至
/// mock / replay / proxy 三条执行路径；handler 定义在 platform-api 而非 sandbox crate，
/// 以避免 sandbox ↔ platform-api 的循环依赖
pub async fn sandbox_handler(
    State(state): State<AppState>,
    req: Request,
) -> Result<impl IntoResponse, AppError> {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let headers = req.headers().clone();

    // 读取沙箱模式，缺失时默认 mock，使开发者可以不带 header 直接获取模拟数据
    let mode = headers
        .get("X-Sandbox-Mode")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("mock");

    // session_id 仅 replay/proxy 模式使用；mock 模式也可选传入以读取 fixed_response 配置
    let session_id = headers
        .get("X-Sandbox-Session")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok());

    // 去掉 /sandbox 前缀后再匹配，使路由定义与沙箱挂载点解耦，与 /gw 前缀逻辑一致
    let path = uri.path().strip_prefix("/sandbox").unwrap_or(uri.path());

    // 路由匹配：沙箱与网关共享同一套 DynamicRouter，确保沙箱覆盖真实路由集合
    let (route_id, path_params) = state
        .router
        .match_route(&method, path)
        .ok_or_else(|| AppError::NotFound(format!("No route matches {method} {path}")))?;

    // 限制请求体大小为 10MB，与网关 handler 保持一致
    let body_bytes = axum::body::to_bytes(req.into_body(), 10 * 1024 * 1024)
        .await
        .map_err(|e| AppError::BadRequest(format!("Failed to read body: {e}")))?;

    let body: Option<serde_json::Value> = if body_bytes.is_empty() {
        None
    } else {
        // 优先解析 JSON；非 JSON 体降级为 String，保持与网关 handler 一致的容错策略
        Some(
            serde_json::from_slice(&body_bytes).unwrap_or_else(|_| {
                serde_json::Value::String(String::from_utf8_lossy(&body_bytes).to_string())
            }),
        )
    };

    match mode {
        "mock" => {
            // 读取路由的 response_schema 用于生成结构匹配的模拟数据
            let route = state.repo.get_route(route_id).await?;

            // 若传入了 session_id，尝试读取 fixed_response 配置；
            // 会话不存在时静默忽略，退回到 schema 推断，避免因过期会话 id 导致 mock 完全失败
            let session_config = if let Some(sid) = session_id {
                state
                    .repo
                    .get_sandbox_session(sid)
                    .await
                    .map(|s| s.config)
                    .unwrap_or(serde_json::json!({}))
            } else {
                serde_json::json!({})
            };

            let mock_data = MockLayer::generate(&route.response_schema, &session_config);
            Ok((StatusCode::OK, Json(mock_data)).into_response())
        }
        "replay" => {
            // replay 和 proxy 模式必须关联会话才有语义，否则无法定位录音集或租户隔离信息
            let sid = session_id.ok_or_else(|| {
                AppError::BadRequest(
                    "X-Sandbox-Session header required for replay mode".into(),
                )
            })?;
            let request_value = body.unwrap_or(serde_json::json!({}));
            let response =
                ReplayLayer::replay(state.repo.as_ref(), sid, route_id, &request_value).await?;
            Ok((StatusCode::OK, Json(response)).into_response())
        }
        "proxy" => {
            let sid = session_id.ok_or_else(|| {
                AppError::BadRequest(
                    "X-Sandbox-Session header required for proxy mode".into(),
                )
            })?;
            let session = state.repo.get_sandbox_session(sid).await?;

            // dispatcher 存在性校验与网关 handler 逻辑一致：路由存在但 dispatcher 缺失
            // 表示该路由协议不被网关支持（如 Http 协议），此时返回 Internal 而非 NotFound
            let dispatcher = state
                .dispatchers
                .get(&route_id)
                .ok_or_else(|| AppError::Internal(format!("No dispatcher for route {route_id}")))?;

            // query_params 在沙箱场景下目前不做解析；代理转发时后端可通过 path 自行提取
            let gateway_req = GatewayRequest {
                route_id,
                method,
                path: path.to_string(),
                headers,
                query_params: HashMap::new(),
                path_params,
                body,
                trace_id: "sandbox".to_string(),
            };

            let resp = ProxyLayer::proxy(dispatcher.value().as_ref(), &session, gateway_req).await?;
            Ok((
                StatusCode::from_u16(resp.status_code).unwrap_or(StatusCode::OK),
                Json(resp.body),
            )
                .into_response())
        }
        other => Err(AppError::BadRequest(format!(
            "Invalid sandbox mode: '{}'. Valid values: mock, replay, proxy",
            other
        ))),
    }
}
