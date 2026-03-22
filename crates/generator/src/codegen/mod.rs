pub mod compiler;
pub mod prompts;
pub mod scaffold;

use crate::llm::client::LlmClient;
use anyhow::Result;
use std::path::PathBuf;
use uuid::Uuid;

/// 代码生成引擎的完整输出产物
pub struct CodegenResult {
    /// 编译后的 .so/.dylib 路径，可被 PluginManager 热加载
    pub plugin_path: PathBuf,
    /// LLM 生成（或修正后）的 Rust 源码，存入 Artifact 表以便审计和重新编译
    pub source_code: String,
    /// 自动生成的 OpenAPI 3.0.3 规范
    pub openapi_spec: serde_json::Value,
    /// LLM 生成的影子测试代码
    pub test_code: String,
    /// 从生成代码中提取的路由列表
    pub routes: Vec<GeneratedRoute>,
}

/// 单个生成的路由描述
pub struct GeneratedRoute {
    pub method: String,
    pub path: String,
    pub operation_name: String,
    pub request_schema: serde_json::Value,
    pub response_schema: serde_json::Value,
    pub description: String,
}

/// 代码生成引擎核心。
///
/// 接受一个 LLM 客户端引用和工作目录，执行 7 阶段流水线：
/// 输入解析 -> LLM 生成 Rust 代码 -> 编译 -> 测试生成 -> 文档生成 -> 观测注入 -> 产物准备
pub struct CodegenEngine<'a> {
    llm: &'a dyn LlmClient,
    workspace_dir: PathBuf,
    /// plugin-sdk crate 的路径，用于生成 Cargo.toml 中的 path 依赖
    sdk_path: PathBuf,
}

impl<'a> CodegenEngine<'a> {
    pub fn new(llm: &'a dyn LlmClient, workspace_dir: PathBuf, sdk_path: PathBuf) -> Self {
        Self {
            llm,
            workspace_dir,
            sdk_path,
        }
    }

    /// 执行完整的 7 阶段代码生成流水线。
    ///
    /// 各阶段设计为顺序执行而非并行，因为后续阶段依赖前置阶段的输出。
    /// Stage 4/5 的失败不会中断整个流程——测试代码和路由提取是辅助产物，
    /// 核心价值在于 Stage 2-3 生成并编译的插件二进制。
    pub async fn generate(
        &self,
        interface_type: &str,
        input_content: &str,
        project_name: &str,
    ) -> Result<CodegenResult> {
        let plugin_id = &Uuid::new_v4().to_string()[..8];
        let safe_name = project_name.replace(' ', "-").to_lowercase();
        let crate_name = format!("plugin-{}-{}", safe_name, plugin_id);
        let crate_dir = self.workspace_dir.join(&crate_name);

        // Stage 1: 输入解析 — 将原始内容直接传给 LLM，由 LLM 理解接口语义
        tracing::info!(interface_type, "Stage 1: Input analysis");

        // Stage 2: LLM 生成 Rust 代码
        tracing::info!("Stage 2: LLM generating Rust source code");
        let prompt = prompts::build_codegen_prompt(interface_type, input_content);
        let raw_response = self
            .llm
            .complete(prompts::SYSTEM_PROMPT, &prompt)
            .await?;
        let source_code = extract_rust_code(&raw_response);

        // 安全检查：警告生成代码中的 unsafe 块（不阻断，仅记录）
        if source_code.contains("unsafe") {
            tracing::warn!("Generated code contains 'unsafe' blocks — review recommended");
        }

        // Stage 3: 编译（失败时 LLM 自动修正，最多重试 3 次）
        tracing::info!("Stage 3: Compiling generated code to plugin");
        scaffold::create_plugin_crate(&crate_dir, &crate_name, &source_code, &self.sdk_path)?;

        let (plugin_path, final_code) =
            compiler::compile_with_llm_fix(&crate_dir, &source_code, self.llm, 3).await?;

        // Stage 4: LLM 生成影子测试
        tracing::info!("Stage 4: Generating shadow tests");
        let test_prompt = prompts::build_test_prompt(&final_code);
        let test_code = self
            .llm
            .complete(
                "You are a Rust test code generator. Generate comprehensive tests for the given plugin code. Output ONLY a ```rust code block.",
                &test_prompt,
            )
            .await
            .unwrap_or_default();

        // Stage 5: 从生成代码中提取路由信息并构建 OpenAPI 规范
        tracing::info!("Stage 5: Generating OpenAPI spec");
        let routes_prompt = prompts::build_routes_prompt(&final_code);
        let routes_json = self
            .llm
            .complete_json(
                "Extract API routes from this Rust plugin code. Return ONLY a JSON array with method, path, name, description, request_schema, response_schema fields.",
                &routes_prompt,
            )
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "Failed to extract routes from LLM, using empty routes");
                serde_json::json!([])
            });

        let routes = parse_routes(&routes_json);
        let openapi = build_openapi_from_routes(&routes, project_name);

        // Stage 6: 观测注入 — prompt 中已要求 LLM 注入 #[tracing::instrument]，无需额外处理

        // Stage 7: 产物准备完成
        tracing::info!(
            plugin_path = %plugin_path.display(),
            routes_count = routes.len(),
            "Stage 7: Artifact ready"
        );

        Ok(CodegenResult {
            plugin_path,
            source_code: final_code,
            openapi_spec: openapi,
            test_code: extract_rust_code(&test_code),
            routes,
        })
    }
}

/// 从 LLM 响应文本中提取 ```rust ... ``` 代码块。
///
/// LLM 回复通常包含 markdown 格式的代码块，需要剥离外层标记。
/// 按优先级匹配：```rust -> 通用 ``` -> 原文返回。
pub fn extract_rust_code(text: &str) -> String {
    // 收集所有 ```rust 代码块，取最长的（通常是主代码，短的可能是示例片段）
    let mut best_block = String::new();
    let mut search_from = 0;

    while let Some(start) = text[search_from..].find("```rust") {
        let abs_start = search_from + start;
        let content_start = abs_start + "```rust".len();
        if let Some(end) = text[content_start..].find("```") {
            let block = text[content_start..content_start + end].trim().to_string();
            // 验证这确实是 Rust 代码（包含 fn/use/struct/impl 等关键字）
            if (block.contains("fn ") || block.contains("use ") || block.contains("struct ") || block.contains("impl "))
                && block.len() > best_block.len()
            {
                best_block = block;
            }
            search_from = content_start + end + 3;
        } else {
            break;
        }
    }

    if !best_block.is_empty() {
        return best_block;
    }

    // 没有 ```rust 标记时，尝试通用 ``` 并检查内容是否像 Rust
    let mut search_from = 0;
    while let Some(start) = text[search_from..].find("```") {
        let abs_start = search_from + start;
        let content_start = abs_start + "```".len();
        // 跳过语言标识行
        let content_start = text[content_start..]
            .find('\n')
            .map(|n| content_start + n + 1)
            .unwrap_or(content_start);
        if let Some(end) = text[content_start..].find("```") {
            let block = text[content_start..content_start + end].trim().to_string();
            if (block.contains("fn ") || block.contains("use ") || block.contains("export_plugin!"))
                && block.len() > best_block.len()
            {
                best_block = block;
            }
            search_from = content_start + end + 3;
        } else {
            break;
        }
    }

    if !best_block.is_empty() {
        return best_block;
    }

    // 无代码块标记时原文返回
    text.trim().to_string()
}

/// 从 LLM 返回的 JSON 中解析路由信息。
/// 字段缺失时使用安全的默认值，保证单个路由解析失败不影响其他路由。
pub fn parse_routes(json: &serde_json::Value) -> Vec<GeneratedRoute> {
    if let Some(arr) = json.as_array() {
        arr.iter()
            .filter_map(|r| {
                Some(GeneratedRoute {
                    method: r["method"].as_str()?.to_string(),
                    path: r["path"].as_str()?.to_string(),
                    operation_name: r["name"].as_str().unwrap_or("unknown").to_string(),
                    request_schema: r
                        .get("request_schema")
                        .cloned()
                        .unwrap_or(serde_json::json!({})),
                    response_schema: r
                        .get("response_schema")
                        .cloned()
                        .unwrap_or(serde_json::json!({})),
                    description: r["description"].as_str().unwrap_or("").to_string(),
                })
            })
            .collect()
    } else {
        Vec::new()
    }
}

/// 从路由列表构建 OpenAPI 3.0.3 规范。
/// 路径前缀 /gw 使其与网关注册的路由前缀一致。
pub fn build_openapi_from_routes(
    routes: &[GeneratedRoute],
    service_name: &str,
) -> serde_json::Value {
    let mut paths = serde_json::Map::new();

    for route in routes {
        let method = route.method.to_lowercase();
        let mut operation = serde_json::Map::new();
        operation.insert(
            "operationId".into(),
            serde_json::json!(route.operation_name),
        );
        operation.insert("summary".into(), serde_json::json!(route.description));

        // 只有非空的 request_schema 才生成 requestBody，避免 GET 请求出现多余的 body 定义
        if !route.request_schema.is_null() && route.request_schema != serde_json::json!({}) {
            operation.insert(
                "requestBody".into(),
                serde_json::json!({
                    "required": true,
                    "content": { "application/json": { "schema": route.request_schema } }
                }),
            );
        }

        // 固定附加网关层错误码（429/502/503/504），与现有 OpenApiGenerator 行为一致
        operation.insert(
            "responses".into(),
            serde_json::json!({
                "200": {
                    "description": "Success",
                    "content": { "application/json": { "schema": route.response_schema } }
                },
                "429": { "description": "Rate limited" },
                "502": { "description": "Backend error" },
                "503": { "description": "Circuit breaker open" },
                "504": { "description": "Backend timeout" }
            }),
        );

        let gw_path = format!("/gw{}", route.path);
        let entry = paths
            .entry(gw_path)
            .or_insert_with(|| serde_json::json!({}));
        if let serde_json::Value::Object(map) = entry {
            map.insert(method, serde_json::Value::Object(operation));
        }
    }

    serde_json::json!({
        "openapi": "3.0.3",
        "info": {
            "title": format!("{} API", service_name),
            "version": "1.0.0"
        },
        "paths": paths
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_rust_code_from_fenced_block() {
        let text = "Here is the code:\n```rust\nfn main() {}\n```\n";
        assert_eq!(extract_rust_code(text), "fn main() {}");
    }

    #[test]
    fn extract_rust_code_from_generic_fence() {
        let text = "```\nfn main() {}\n```";
        assert_eq!(extract_rust_code(text), "fn main() {}");
    }

    #[test]
    fn extract_rust_code_plain_text() {
        let text = "fn main() {}";
        assert_eq!(extract_rust_code(text), "fn main() {}");
    }

    #[test]
    fn extract_rust_code_with_surrounding_text() {
        let text = "Some explanation\n```rust\nuse std::io;\nfn main() {\n    println!(\"hello\");\n}\n```\nMore text";
        let code = extract_rust_code(text);
        assert!(code.starts_with("use std::io;"));
        assert!(code.ends_with('}'));
    }

    #[test]
    fn parse_routes_from_valid_json() {
        let json = serde_json::json!([
            {
                "method": "POST",
                "path": "/api/v1/calc/add",
                "name": "add",
                "description": "Add two numbers",
                "request_schema": {"type": "object"},
                "response_schema": {"type": "object"}
            }
        ]);
        let routes = parse_routes(&json);
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].method, "POST");
        assert_eq!(routes[0].path, "/api/v1/calc/add");
    }

    #[test]
    fn parse_routes_skips_invalid_entries() {
        // 缺少 method 字段的条目应被跳过
        let json = serde_json::json!([
            {"path": "/api/v1/calc/add"},
            {"method": "GET", "path": "/api/v1/calc/history"}
        ]);
        let routes = parse_routes(&json);
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].method, "GET");
    }

    #[test]
    fn parse_routes_returns_empty_for_non_array() {
        let json = serde_json::json!({"error": "not an array"});
        let routes = parse_routes(&json);
        assert!(routes.is_empty());
    }

    #[test]
    fn build_openapi_generates_valid_structure() {
        let routes = vec![GeneratedRoute {
            method: "POST".to_string(),
            path: "/api/v1/calc/add".to_string(),
            operation_name: "add".to_string(),
            request_schema: serde_json::json!({"type": "object"}),
            response_schema: serde_json::json!({"type": "object"}),
            description: "Add numbers".to_string(),
        }];
        let spec = build_openapi_from_routes(&routes, "Calculator");
        assert_eq!(spec["openapi"], "3.0.3");
        assert!(spec["info"]["title"].as_str().unwrap().contains("Calculator"));
        assert!(spec["paths"]["/gw/api/v1/calc/add"]["post"].is_object());
    }

    #[test]
    fn build_openapi_skips_request_body_for_empty_schema() {
        let routes = vec![GeneratedRoute {
            method: "GET".to_string(),
            path: "/api/v1/items".to_string(),
            operation_name: "listItems".to_string(),
            request_schema: serde_json::json!({}),
            response_schema: serde_json::json!({"type": "array"}),
            description: "List items".to_string(),
        }];
        let spec = build_openapi_from_routes(&routes, "Items");
        let get_op = &spec["paths"]["/gw/api/v1/items"]["get"];
        assert!(get_op["requestBody"].is_null());
    }
}
