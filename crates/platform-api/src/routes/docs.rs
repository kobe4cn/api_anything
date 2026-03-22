use axum::extract::{Path, State};
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

/// 根据语言参数动态生成对应的 SDK 客户端代码；
/// 基于当前活跃路由的 schema 信息模板化生成，无需外部代码生成工具
pub async fn generate_sdk(
    State(state): State<AppState>,
    Path(language): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let routes = state.repo.list_active_routes_with_bindings().await?;
    let code = match language.as_str() {
        "typescript" => generate_typescript_sdk(&routes),
        "python" => generate_python_sdk(&routes),
        "java" => generate_java_sdk(&routes),
        "go" => generate_go_sdk(&routes),
        _ => return Err(AppError::BadRequest(format!("Unsupported language: {}. Supported: typescript, python, java, go", language))),
    };
    Ok((StatusCode::OK, [("content-type", "text/plain")], code))
}

/// 将路由路径转为合法的函数名：method + path，斜杠替换为下划线
fn route_to_fn_name(method: &HttpMethod, path: &str) -> String {
    let method_str = match method {
        HttpMethod::Get => "get",
        HttpMethod::Post => "post",
        HttpMethod::Put => "put",
        HttpMethod::Patch => "patch",
        HttpMethod::Delete => "delete",
    };
    let path_part = path
        .trim_start_matches('/')
        .replace('/', "_")
        .replace('-', "_");
    format!("{}_{}", method_str, path_part)
}

/// 从 JSON Schema 中提取 properties 字段列表及其类型，
/// 用于 SDK 函数签名中的参数定义
fn extract_params(schema: &Value) -> Vec<(String, String)> {
    let props = schema.get("properties")
        .or_else(|| schema.get("content")
            .and_then(|c| c.get("application/json"))
            .and_then(|j| j.get("schema"))
            .and_then(|s| s.get("properties"))
        );
    if let Some(Value::Object(map)) = props {
        map.iter()
            .map(|(name, prop)| {
                let type_name = prop.get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("any")
                    .to_string();
                (name.clone(), type_name)
            })
            .collect()
    } else {
        Vec::new()
    }
}

fn generate_typescript_sdk(routes: &[RouteWithBinding]) -> String {
    let mut code = String::new();
    code.push_str("// Auto-generated API-Anything SDK for TypeScript\n");
    code.push_str("// Generated from active route definitions\n\n");
    code.push_str("const BASE_URL = \"http://localhost:8080/gw\";\n\n");

    for route in routes {
        let fn_name = route_to_fn_name(&route.method, &route.path);
        let method_str = format!("{:?}", route.method).to_uppercase();
        let params = extract_params(&route.request_schema);

        // 生成 TypeScript 接口和函数
        if params.is_empty() {
            code.push_str(&format!(
                "export async function {fn_name}() {{\n"
            ));
        } else {
            let ts_params: Vec<String> = params.iter().map(|(name, typ)| {
                let ts_type = match typ.as_str() {
                    "integer" | "number" => "number",
                    "boolean" => "boolean",
                    "array" => "any[]",
                    _ => "string",
                };
                format!("{}: {}", name, ts_type)
            }).collect();
            code.push_str(&format!(
                "export async function {fn_name}(body: {{{params}}}) {{\n",
                fn_name = fn_name,
                params = ts_params.join(", "),
            ));
        }

        // fetch 调用
        if method_str == "GET" || method_str == "DELETE" {
            code.push_str(&format!(
                "  const resp = await fetch(`${{BASE_URL}}{path}`, {{\n    method: \"{method}\"\n  }});\n",
                path = route.path,
                method = method_str,
            ));
        } else {
            code.push_str(&format!(
                "  const resp = await fetch(`${{BASE_URL}}{path}`, {{\n    method: \"{method}\",\n    headers: {{\"Content-Type\": \"application/json\"}},\n    body: JSON.stringify(body)\n  }});\n",
                path = route.path,
                method = method_str,
            ));
        }
        code.push_str("  return resp.json();\n}\n\n");
    }

    code
}

fn generate_python_sdk(routes: &[RouteWithBinding]) -> String {
    let mut code = String::new();
    code.push_str("# Auto-generated API-Anything SDK for Python\n");
    code.push_str("# Generated from active route definitions\n\n");
    code.push_str("import requests\n\n");
    code.push_str("BASE_URL = \"http://localhost:8080/gw\"\n\n");

    for route in routes {
        let fn_name = route_to_fn_name(&route.method, &route.path);
        let method_str = format!("{:?}", route.method).to_lowercase();
        let params = extract_params(&route.request_schema);

        if params.is_empty() {
            code.push_str(&format!("def {fn_name}():\n"));
        } else {
            let py_params: Vec<String> = params.iter().map(|(name, typ)| {
                let py_type = match typ.as_str() {
                    "integer" => "int",
                    "number" => "float",
                    "boolean" => "bool",
                    "array" => "list",
                    _ => "str",
                };
                format!("{}: {}", name, py_type)
            }).collect();
            code.push_str(&format!(
                "def {fn_name}({params}):\n",
                fn_name = fn_name,
                params = py_params.join(", "),
            ));
        }

        // requests 调用
        if method_str == "get" || method_str == "delete" {
            code.push_str(&format!(
                "    resp = requests.{method}(f\"{{BASE_URL}}{path}\")\n",
                method = method_str,
                path = route.path,
            ));
        } else {
            let json_body = if params.is_empty() {
                "{}".to_string()
            } else {
                let fields: Vec<String> = params.iter()
                    .map(|(name, _)| format!("\"{name}\": {name}"))
                    .collect();
                format!("{{{}}}", fields.join(", "))
            };
            code.push_str(&format!(
                "    resp = requests.{method}(f\"{{BASE_URL}}{path}\", json={json_body})\n",
                method = method_str,
                path = route.path,
                json_body = json_body,
            ));
        }
        code.push_str("    return resp.json()\n\n");
    }

    code
}

fn generate_java_sdk(routes: &[RouteWithBinding]) -> String {
    let mut code = String::new();
    code.push_str("// Auto-generated API-Anything SDK for Java\n");
    code.push_str("// Generated from active route definitions\n\n");
    code.push_str("import java.net.URI;\n");
    code.push_str("import java.net.http.HttpClient;\n");
    code.push_str("import java.net.http.HttpRequest;\n");
    code.push_str("import java.net.http.HttpResponse;\n\n");
    code.push_str("public class ApiAnythingClient {\n");
    code.push_str("    private static final String BASE_URL = \"http://localhost:8080/gw\";\n");
    code.push_str("    private final HttpClient client = HttpClient.newHttpClient();\n\n");

    for route in routes {
        let fn_name = route_to_fn_name(&route.method, &route.path);
        let method_str = format!("{:?}", route.method).to_uppercase();
        let params = extract_params(&route.request_schema);

        let java_params: Vec<String> = params.iter().map(|(name, typ)| {
            let java_type = match typ.as_str() {
                "integer" => "int",
                "number" => "double",
                "boolean" => "boolean",
                _ => "String",
            };
            format!("{} {}", java_type, name)
        }).collect();

        code.push_str(&format!(
            "    public String {fn_name}({params}) throws Exception {{\n",
            fn_name = fn_name,
            params = java_params.join(", "),
        ));

        if method_str == "GET" || method_str == "DELETE" {
            code.push_str(&format!(
                "        HttpRequest request = HttpRequest.newBuilder()\n            .uri(URI.create(BASE_URL + \"{path}\"))\n            .method(\"{method}\", HttpRequest.BodyPublishers.noBody())\n            .build();\n",
                path = route.path,
                method = method_str,
            ));
        } else {
            code.push_str(&format!(
                "        HttpRequest request = HttpRequest.newBuilder()\n            .uri(URI.create(BASE_URL + \"{path}\"))\n            .header(\"Content-Type\", \"application/json\")\n            .method(\"{method}\", HttpRequest.BodyPublishers.ofString(body))\n            .build();\n",
                path = route.path,
                method = method_str,
            ));
        }
        code.push_str("        return client.send(request, HttpResponse.BodyHandlers.ofString()).body();\n");
        code.push_str("    }\n\n");
    }

    code.push_str("}\n");
    code
}

fn generate_go_sdk(routes: &[RouteWithBinding]) -> String {
    let mut code = String::new();
    code.push_str("// Auto-generated API-Anything SDK for Go\n");
    code.push_str("// Generated from active route definitions\n\n");
    code.push_str("package apianything\n\n");
    code.push_str("import (\n\t\"bytes\"\n\t\"encoding/json\"\n\t\"io\"\n\t\"net/http\"\n)\n\n");
    code.push_str("const BaseURL = \"http://localhost:8080/gw\"\n\n");

    for route in routes {
        let fn_name = route_to_fn_name(&route.method, &route.path);
        // Go 函数名首字母大写以导出
        let fn_name = fn_name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default() + &fn_name[1..];
        let method_str = format!("{:?}", route.method).to_uppercase();

        code.push_str(&format!(
            "func {fn_name}(body map[string]interface{{}}) (map[string]interface{{}}, error) {{\n"
        ));

        if method_str == "GET" || method_str == "DELETE" {
            code.push_str(&format!(
                "\treq, err := http.NewRequest(\"{method}\", BaseURL+\"{path}\", nil)\n",
                method = method_str,
                path = route.path,
            ));
        } else {
            code.push_str("\tdata, _ := json.Marshal(body)\n");
            code.push_str(&format!(
                "\treq, err := http.NewRequest(\"{method}\", BaseURL+\"{path}\", bytes.NewReader(data))\n",
                method = method_str,
                path = route.path,
            ));
            code.push_str("\treq.Header.Set(\"Content-Type\", \"application/json\")\n");
        }
        code.push_str("\tif err != nil { return nil, err }\n");
        code.push_str("\tresp, err := http.DefaultClient.Do(req)\n");
        code.push_str("\tif err != nil { return nil, err }\n");
        code.push_str("\tdefer resp.Body.Close()\n");
        code.push_str("\trespBody, _ := io.ReadAll(resp.Body)\n");
        code.push_str("\tvar result map[string]interface{}\n");
        code.push_str("\tjson.Unmarshal(respBody, &result)\n");
        code.push_str("\treturn result, nil\n}\n\n");
    }

    code
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
