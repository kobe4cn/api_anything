use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use api_anything_common::error::AppError;
use api_anything_common::models::{HttpMethod, RouteWithBinding};
use api_anything_metadata::MetadataRepo;
use serde_json::{json, Value};

use crate::state::AppState;

/// 动态从数据库中激活路由生成 OpenAPI 3.0 规范；
/// 每次请求都重新生成以确保文档与当前路由状态保持一致，不存在缓存失效问题
pub async fn openapi_json(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let routes = state.repo.list_active_routes_with_bindings().await?;
    let spec = build_openapi_from_routes(&routes);
    Ok((
        StatusCode::OK,
        [("content-type", "application/json")],
        serde_json::to_string_pretty(&spec).unwrap_or_default(),
    ))
}

/// 直接从路由元数据构造 OpenAPI 3.0 规范，绕过 UnifiedContract 中间表示；
/// 网关路径统一挂在 /gw 前缀下，与 gateway_handler 的通配规则对应
fn build_openapi_from_routes(routes: &[RouteWithBinding]) -> serde_json::Value {
    let mut paths = serde_json::Map::new();

    for route in routes {
        let method = match route.method {
            HttpMethod::Get => "get",
            HttpMethod::Post => "post",
            HttpMethod::Put => "put",
            HttpMethod::Patch => "patch",
            HttpMethod::Delete => "delete",
        };

        let mut operation = serde_json::Map::new();
        operation.insert(
            "operationId".into(),
            json!(format!("{}_{}", method, route.path.replace('/', "_"))),
        );
        // 使用协议类型作为 tag，方便在 Swagger UI 中按 SOAP/HTTP/CLI 等分组展示
        operation.insert("tags".into(), json!([format!("{:?}", route.protocol)]));

        // 仅在路由定义了非空 request_schema 时才生成 requestBody，
        // 避免 GET 类接口出现多余的请求体声明
        if !route.request_schema.is_null() && route.request_schema != json!({}) {
            operation.insert(
                "requestBody".into(),
                json!({
                    "required": true,
                    "content": { "application/json": { "schema": route.request_schema } }
                }),
            );
        }

        let mut responses = serde_json::Map::new();
        if !route.response_schema.is_null() && route.response_schema != json!({}) {
            responses.insert(
                "200".into(),
                json!({
                    "description": "Successful response",
                    "content": { "application/json": { "schema": route.response_schema } }
                }),
            );
        } else {
            responses.insert("200".into(), json!({"description": "Successful response"}));
        }
        // 标准网关错误码：限流、后端错误、熔断、超时
        responses.insert("429".into(), json!({"description": "Rate limited"}));
        responses.insert("502".into(), json!({"description": "Backend error"}));
        responses.insert("503".into(), json!({"description": "Circuit breaker open"}));
        responses.insert("504".into(), json!({"description": "Backend timeout"}));
        operation.insert("responses".into(), Value::Object(responses));

        // 网关统一在 /gw 前缀下暴露外部路径，与路由表中存储的内部路径区分
        let gw_path = format!("/gw{}", route.path);
        let entry = paths.entry(gw_path).or_insert_with(|| json!({}));
        if let Value::Object(map) = entry {
            map.insert(method.into(), Value::Object(operation));
        }
    }

    json!({
        "openapi": "3.0.3",
        "info": {
            "title": "API-Anything Gateway",
            "description": "Auto-generated REST API gateway for legacy systems",
            "version": "1.0.0"
        },
        "servers": [{"url": "/", "description": "Gateway"}],
        "paths": paths
    })
}

/// Swagger UI 静态页面：从 CDN 加载资源以避免在服务端存储前端资源，
/// topbar 隐藏减少视觉噪音，保持与平台品牌一致
pub async fn swagger_ui() -> impl IntoResponse {
    let html = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <title>API-Anything - API Documentation</title>
    <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui.css">
    <style>body { margin: 0; } .topbar { display: none; }</style>
</head>
<body>
    <div id="swagger-ui"></div>
    <script src="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
    <script>
        SwaggerUIBundle({
            url: '/api/v1/docs/openapi.json',
            dom_id: '#swagger-ui',
            presets: [SwaggerUIBundle.presets.apis, SwaggerUIBundle.SwaggerUIStandalonePreset],
            layout: 'BaseLayout'
        });
    </script>
</body>
</html>"#;
    axum::response::Html(html)
}

/// 为 AI Agent 生成结构化的 Markdown 提示词；
/// 包含所有激活路由的请求/响应 schema，使 Agent 无需额外查阅文档即可调用网关
pub async fn agent_prompt(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let routes = state.repo.list_active_routes_with_bindings().await?;
    let prompt = build_agent_prompt(&routes);
    Ok((
        StatusCode::OK,
        [("content-type", "text/markdown")],
        prompt,
    ))
}

fn build_agent_prompt(routes: &[RouteWithBinding]) -> String {
    let mut prompt = String::new();
    prompt.push_str("# API-Anything Gateway\n\n");
    prompt.push_str("Available API endpoints:\n\n");

    for route in routes {
        // Debug 格式输出枚举变体名，转大写后与 HTTP 规范一致
        let method = format!("{:?}", route.method).to_uppercase();
        prompt.push_str(&format!("## {} /gw{}\n", method, route.path));
        prompt.push_str(&format!("- **Protocol:** {:?}\n", route.protocol));

        // 只有路由定义了实际 schema 时才输出，避免 Agent 看到无意义的空 schema
        if !route.request_schema.is_null() && route.request_schema != json!({}) {
            prompt.push_str("- **Request Body:**\n```json\n");
            prompt.push_str(
                &serde_json::to_string_pretty(&route.request_schema).unwrap_or_default(),
            );
            prompt.push_str("\n```\n");
        }

        if !route.response_schema.is_null() && route.response_schema != json!({}) {
            prompt.push_str("- **Response:**\n```json\n");
            prompt.push_str(
                &serde_json::to_string_pretty(&route.response_schema).unwrap_or_default(),
            );
            prompt.push_str("\n```\n");
        }
        prompt.push('\n');
    }

    prompt
}
