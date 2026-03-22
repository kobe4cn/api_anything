use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use api_anything_common::error::AppError;
use api_anything_common::models::DeliveryStatus;
use api_anything_compensation::request_logger::RequestLogger;
use api_anything_gateway::types::GatewayRequest;
use api_anything_metadata::repo::MetadataRepo;
use serde_json::json;
use std::collections::HashMap;
use crate::state::AppState;

/// 网关通配 handler — 接收所有 /gw/* 请求，通过 DynamicRouter 匹配路由后分发至后端
/// path_params 由路由匹配阶段填充，避免在 handler 中重新解析 URI 模板
pub async fn gateway_handler(
    State(state): State<AppState>,
    req: Request,
) -> Result<impl IntoResponse, AppError> {
    let method = req.method().clone();
    let uri = req.uri().clone();
    // 去掉 /gw 前缀后再匹配，使路由定义与网关挂载点解耦
    let path = uri.path().strip_prefix("/gw").unwrap_or(uri.path());
    let headers = req.headers().clone();

    // 手动解析 query string，避免引入额外依赖；空 query 时返回空 map
    let query_params: HashMap<String, String> = uri
        .query()
        .map(|q| {
            q.split('&')
                .filter_map(|pair| {
                    let mut parts = pair.splitn(2, '=');
                    Some((
                        parts.next()?.to_string(),
                        parts.next().unwrap_or("").to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();

    // 限制请求体大小为 10MB，防止大包攻击耗尽内存
    let body_bytes = axum::body::to_bytes(req.into_body(), 10 * 1024 * 1024)
        .await
        .map_err(|e| AppError::BadRequest(format!("Failed to read body: {e}")))?;

    let body = if body_bytes.is_empty() {
        None
    } else {
        // 优先尝试解析为 JSON；非 JSON 体（如纯文本）降级为 String 类型的 Value
        Some(
            serde_json::from_slice(&body_bytes).unwrap_or_else(|_| {
                serde_json::Value::String(
                    String::from_utf8_lossy(&body_bytes).to_string(),
                )
            }),
        )
    };

    // 1. 路由匹配：找不到路由时返回 404，而非 500，明确区分配置缺失与服务内部错误
    let (route_id, path_params) = state
        .router
        .match_route(&method, path)
        .ok_or_else(|| AppError::NotFound(format!("No route matches {method} {path}")))?;

    // 2. 查找 dispatcher：路由存在但 dispatcher 缺失表示网关初始化未完成
    let dispatcher = state
        .dispatchers
        .get(&route_id)
        .ok_or_else(|| AppError::Internal(format!("No dispatcher for route {route_id}")))?;

    // 3. 从请求头中提取 traceparent，用于分布式链路追踪；缺失时使用占位符而非失败
    let trace_id = headers
        .get("traceparent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    // 4. 查询路由配置以获取 delivery_guarantee；
    //    此处单独查询而非从路由表缓存读取，确保使用最新的数据库配置
    let route = state.repo.get_route(route_id).await?;

    // 5. 从 Idempotency-Key 头提取幂等键，ExactlyOnce 语义下必须存在；
    //    提前转为 owned String，避免后续 headers 被移入 GatewayRequest 后仍持有借用
    let idempotency_key: Option<String> = headers
        .get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned());

    // 6. 根据 delivery_guarantee 决定是否持久化投递记录；
    //    AtMostOnce 不记录（发即忘），AtLeastOnce/ExactlyOnce 写入 delivery_records
    let log_result = RequestLogger::log_if_needed(
        state.repo.as_ref(),
        &route.delivery_guarantee,
        route_id,
        &trace_id,
        idempotency_key.as_deref(),
        &body.clone().unwrap_or(serde_json::json!({})),
    ).await;

    // AlreadyDelivered 是幂等键命中的正常路径，直接返回 200 告知调用方无需重试；
    // 其他错误（DB 故障、缺少 Idempotency-Key 等）向上冒泡为 HTTP 错误
    let delivery_record_id = match log_result {
        Ok(record_opt) => record_opt.map(|r| r.id),
        Err(AppError::AlreadyDelivered) => {
            return Ok((StatusCode::OK, Json(json!({"status": "already_delivered"}))).into_response());
        }
        Err(e) => return Err(e),
    };

    let gateway_req = GatewayRequest {
        route_id,
        method,
        path: path.to_string(),
        headers,
        query_params,
        path_params,
        body,
        trace_id,
    };

    // 7. 分发请求：dispatcher 内部按限流→熔断→超时顺序执行保护逻辑
    match dispatcher.dispatch(gateway_req).await {
        Ok(resp) => {
            // 投递成功后异步更新状态；使用 let _ 忽略错误，确保状态更新失败不影响正常响应
            if let Some(record_id) = delivery_record_id {
                let _ = state.repo.update_delivery_status(
                    record_id,
                    DeliveryStatus::Delivered,
                    None,
                    None,
                ).await;
                if let Some(key) = &idempotency_key {
                    let _ = state.repo.mark_idempotency_delivered(key, "success").await;
                }
            }
            Ok((
                StatusCode::from_u16(resp.status_code).unwrap_or(StatusCode::OK),
                Json(resp.body),
            ).into_response())
        }
        Err(e) => {
            // 投递失败时调度下次重试（1 秒后），由 retry_worker 按指数退避接管后续重试
            if let Some(record_id) = delivery_record_id {
                let next_retry = chrono::Utc::now() + chrono::Duration::seconds(1);
                let _ = state.repo.update_delivery_status(
                    record_id,
                    DeliveryStatus::Failed,
                    Some(&format!("{e}")),
                    Some(next_retry),
                ).await;
            }
            Err(e)
        }
    }
}
