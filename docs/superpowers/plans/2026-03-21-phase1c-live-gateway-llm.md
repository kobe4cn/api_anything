# Phase 1c: 网关路由加载 + LLM 增强 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 闭合 Phase 1 的端到端链路 — 网关从元数据自动加载生成的路由并实际代理 SOAP 请求。同时引入 LLM 适配层增强 WSDL 的语义映射能力，添加影子测试和 Agent 提示词生成。

**Architecture:** platform-api 启动时从元数据加载活跃路由，为每条路由创建 SoapAdapter 实例并注册到 DynamicRouter。LLM 适配层为 generator crate 新增多模型客户端（Claude/OpenAI/DeepSeek），通过 trait 抽象实现可插拔。

**Tech Stack:** 已有 crates + reqwest (LLM API 调用), wiremock (mock SOAP 服务端测试)

**Spec:** `docs/superpowers/specs/2026-03-21-api-anything-platform-design.md` §4, §5

---

## File Structure

```
crates/gateway/src/
    └── loader.rs                   # 从元数据加载路由 → 创建适配器 → 注册到 DynamicRouter

crates/generator/src/
    ├── llm/
    │   ├── mod.rs
    │   ├── client.rs               # LlmClient trait
    │   ├── claude.rs               # Claude API 实现
    │   └── openai.rs               # OpenAI API 实现
    ├── wsdl/
    │   └── llm_mapper.rs           # LLM 增强的语义映射器
    ├── shadow_test.rs              # 影子测试生成
    └── agent_prompt.rs             # Agent 提示词生成
```

---

### Task 1: 网关路由加载器 (Gateway Route Loader)

**Files:**
- Create: `crates/gateway/src/loader.rs`
- Modify: `crates/gateway/src/lib.rs`
- Modify: `crates/platform-api/src/main.rs` — 启动时调用加载器

网关启动时从 DB 加载路由，为每条 SOAP 路由创建 SoapAdapter，注册到 DynamicRouter 和 dispatchers DashMap。

- [ ] **Step 1: 实现 RouteLoader**

```rust
use crate::adapters::soap::{SoapAdapter, SoapConfig};
use crate::dispatcher::{BackendDispatcher, ProtectionStack};
use crate::router::{DynamicRouter, RouteTable};
use api_anything_common::models::*;
use api_anything_metadata::MetadataRepo;
use dashmap::DashMap;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

pub struct RouteLoader;

impl RouteLoader {
    /// 从元数据加载所有活跃路由，创建适配器和调度器
    pub async fn load(
        repo: &impl MetadataRepo,
        router: &DynamicRouter,
        dispatchers: &DashMap<Uuid, Arc<BackendDispatcher>>,
    ) -> Result<usize, anyhow::Error> {
        let routes = repo.list_active_routes_with_bindings().await?;
        let mut table = RouteTable::new();
        let mut count = 0;

        for route in &routes {
            // 根据协议类型创建对应的适配器
            let adapter: Box<dyn crate::adapter::ProtocolAdapter> = match route.protocol {
                ProtocolType::Soap => {
                    let config = Self::build_soap_config(route)?;
                    Box::new(SoapAdapter::new(config))
                }
                // 其他协议在后续 Phase 实现
                _ => {
                    tracing::warn!(protocol = ?route.protocol, route_id = %route.route_id, "Unsupported protocol, skipping");
                    continue;
                }
            };

            // 从绑定配置构建保护栈
            let protection = Self::build_protection_stack(route);

            // 创建调度器
            let dispatcher = Arc::new(BackendDispatcher::new(adapter, protection));
            dispatchers.insert(route.route_id, dispatcher);

            // 注册路由
            let method = Self::to_axum_method(&route.method);
            table.insert(method, &route.path, route.route_id);
            count += 1;

            tracing::info!(
                route_id = %route.route_id,
                path = %route.path,
                protocol = ?route.protocol,
                "Loaded route"
            );
        }

        router.update(table);
        tracing::info!(count = count, "Route loading complete");
        Ok(count)
    }

    fn build_soap_config(route: &RouteWithBinding) -> Result<SoapConfig, anyhow::Error> {
        let ec = &route.endpoint_config;
        Ok(SoapConfig {
            endpoint_url: ec["url"].as_str().unwrap_or("").to_string(),
            soap_action: ec["soap_action"].as_str().unwrap_or("").to_string(),
            operation_name: ec["operation_name"].as_str().unwrap_or("").to_string(),
            namespace: ec["namespace"].as_str().unwrap_or("").to_string(),
        })
    }

    fn build_protection_stack(route: &RouteWithBinding) -> ProtectionStack {
        let pool_cfg = &route.connection_pool_config;
        let cb_cfg = &route.circuit_breaker_config;
        let rl_cfg = &route.rate_limit_config;

        let rps = rl_cfg["requests_per_second"].as_u64().unwrap_or(1000) as u32;
        let max_conn = pool_cfg["max_connections"].as_u64().unwrap_or(100) as u32;
        let err_threshold = cb_cfg["error_threshold_percent"].as_f64().unwrap_or(50.0);
        let window_ms = cb_cfg["window_duration_ms"].as_u64().unwrap_or(30000);
        let open_ms = cb_cfg["open_duration_ms"].as_u64().unwrap_or(60000);
        let half_open_max = cb_cfg["half_open_max_requests"].as_u64().unwrap_or(3) as u32;
        let timeout_ms = route.timeout_ms as u64;

        ProtectionStack::new(
            rps, max_conn,
            err_threshold, Duration::from_millis(window_ms),
            Duration::from_millis(open_ms), half_open_max,
            Duration::from_millis(timeout_ms),
        )
    }

    fn to_axum_method(method: &HttpMethod) -> axum::http::Method {
        match method {
            HttpMethod::Get => axum::http::Method::GET,
            HttpMethod::Post => axum::http::Method::POST,
            HttpMethod::Put => axum::http::Method::PUT,
            HttpMethod::Patch => axum::http::Method::PATCH,
            HttpMethod::Delete => axum::http::Method::DELETE,
        }
    }
}
```

- [ ] **Step 2: 更新 platform-api main.rs**

在服务启动时、migrations 之后调用 RouteLoader::load：

```rust
// 加载已有路由到网关
let loaded = api_anything_gateway::loader::RouteLoader::load(
    &repo,
    &state.router,
    &state.dispatchers,
).await?;
tracing::info!(routes = loaded, "Gateway routes loaded from metadata");
```

注意：需要在 build_app 之后、axum::serve 之前执行，因为需要访问 state 中的 router 和 dispatchers。可能需要重构 main.rs，先创建 state，再 build_app(state)，再 load routes。

- [ ] **Step 3: 编写集成测试**

用 wiremock 创建一个 mock SOAP 服务，然后：
1. 调用 CLI generate 解析 WSDL（指向 mock 服务地址）
2. 调用 RouteLoader 加载路由
3. 通过 gateway `/gw/...` 端点发送 JSON 请求
4. 验证 mock SOAP 服务收到了正确的 SOAP XML
5. 验证 gateway 返回了正确的 JSON 响应

- [ ] **Step 4: 运行测试**

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(gateway): add route loader for startup route initialization from metadata"
```

---

### Task 2: LLM 适配层 (Multi-Model Client)

**Files:**
- Create: `crates/generator/src/llm/mod.rs`
- Create: `crates/generator/src/llm/client.rs`
- Create: `crates/generator/src/llm/claude.rs`
- Create: `crates/generator/src/llm/openai.rs`

- [ ] **Step 1: 定义 LlmClient trait**

```rust
use serde_json::Value;

/// LLM 客户端统一接口
/// 支持多模型切换（Claude/OpenAI/DeepSeek 等）
#[trait_variant::make(Send)]
pub trait LlmClient: Send + Sync {
    /// 发送 prompt，返回文本响应
    async fn complete(&self, system_prompt: &str, user_prompt: &str) -> Result<String, anyhow::Error>;

    /// 发送 prompt，要求返回 JSON 格式响应
    async fn complete_json(&self, system_prompt: &str, user_prompt: &str) -> Result<Value, anyhow::Error>;

    /// 模型名称（用于日志）
    fn model_name(&self) -> &str;
}
```

实际上，不要用 trait_variant，直接用 async fn in trait（Rust 1.75+ 支持，我们用的 stable）：

```rust
pub trait LlmClient: Send + Sync {
    fn complete(&self, system_prompt: &str, user_prompt: &str) -> impl std::future::Future<Output = Result<String, anyhow::Error>> + Send;
    fn complete_json(&self, system_prompt: &str, user_prompt: &str) -> impl std::future::Future<Output = Result<Value, anyhow::Error>> + Send;
    fn model_name(&self) -> &str;
}
```

- [ ] **Step 2: 实现 Claude API 客户端**

```rust
pub struct ClaudeClient {
    api_key: String,
    model: String,
    client: reqwest::Client,
}

impl ClaudeClient {
    pub fn new(api_key: String, model: Option<String>) -> Self {
        Self {
            api_key,
            model: model.unwrap_or_else(|| "claude-sonnet-4-20250514".to_string()),
            client: reqwest::Client::new(),
        }
    }
}

impl LlmClient for ClaudeClient {
    async fn complete(&self, system_prompt: &str, user_prompt: &str) -> Result<String, anyhow::Error> {
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 4096,
            "system": system_prompt,
            "messages": [{"role": "user", "content": user_prompt}]
        });

        let resp = self.client.post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let result: Value = resp.json().await?;
        let text = result["content"][0]["text"].as_str()
            .ok_or_else(|| anyhow::anyhow!("No text in Claude response"))?;
        Ok(text.to_string())
    }

    async fn complete_json(&self, system_prompt: &str, user_prompt: &str) -> Result<Value, anyhow::Error> {
        let text = self.complete(system_prompt, user_prompt).await?;
        // 提取 JSON 块（可能被 markdown code fence 包围）
        let json_str = extract_json_block(&text).unwrap_or(&text);
        Ok(serde_json::from_str(json_str)?)
    }

    fn model_name(&self) -> &str { &self.model }
}

fn extract_json_block(text: &str) -> Option<&str> {
    if let Some(start) = text.find("```json") {
        let content_start = start + "```json".len();
        if let Some(end) = text[content_start..].find("```") {
            return Some(text[content_start..content_start + end].trim());
        }
    }
    if text.trim_start().starts_with('{') || text.trim_start().starts_with('[') {
        return Some(text.trim());
    }
    None
}
```

- [ ] **Step 3: 实现 OpenAI API 客户端**

类似结构，调用 `https://api.openai.com/v1/chat/completions`。

- [ ] **Step 4: 编写测试**

Mock HTTP 调用测试 JSON 提取逻辑：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_json_from_code_fence() {
        let text = "Here's the result:\n```json\n{\"name\": \"test\"}\n```\n";
        let json = extract_json_block(text).unwrap();
        let parsed: Value = serde_json::from_str(json).unwrap();
        assert_eq!(parsed["name"], "test");
    }

    #[test]
    fn extracts_raw_json() {
        let text = "{\"name\": \"test\"}";
        let json = extract_json_block(text).unwrap();
        let parsed: Value = serde_json::from_str(json).unwrap();
        assert_eq!(parsed["name"], "test");
    }

    #[test]
    fn returns_none_for_plain_text() {
        assert!(extract_json_block("just some text").is_none());
    }
}
```

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(generator): add LLM adapter layer with Claude and OpenAI clients"
```

---

### Task 3: LLM 增强的 WSDL 语义映射

**Files:**
- Create: `crates/generator/src/wsdl/llm_mapper.rs`
- Modify: `crates/generator/src/wsdl/mod.rs`
- Modify: `crates/generator/src/pipeline.rs`

用 LLM 优化确定性映射的两个弱点：
1. REST 路由命名（`Add` → `POST /calculator/add` 是简单的，但复杂名称如 `getOrdersByCustomerIdAndDateRange` 需要更智能的拆分）
2. HTTP 方法选择（查询操作应为 GET 而非 POST）

- [ ] **Step 1: 实现 LlmEnhancedMapper**

```rust
use crate::llm::client::LlmClient;
use crate::unified_contract::UnifiedContract;
use crate::wsdl::mapper::WsdlMapper;
use crate::wsdl::parser::WsdlDefinition;

pub struct LlmEnhancedMapper;

impl LlmEnhancedMapper {
    /// 先用确定性 mapper 生成基础契约，再用 LLM 优化路由命名和方法选择
    pub async fn map(
        wsdl: &WsdlDefinition,
        llm: &impl LlmClient,
    ) -> Result<UnifiedContract, anyhow::Error> {
        // 1. 确定性映射作为基线
        let mut contract = WsdlMapper::map(wsdl)?;

        // 2. 构建 prompt，让 LLM 优化路由设计
        let operations_summary: Vec<_> = contract.operations.iter().map(|op| {
            serde_json::json!({
                "name": op.name,
                "current_method": op.http_method,
                "current_path": op.path,
                "has_input": op.input.is_some(),
                "has_output": op.output.is_some(),
            })
        }).collect();

        let system_prompt = "You are a REST API design expert. Given SOAP operation names, suggest optimal RESTful HTTP methods and path naming. Respond with a JSON array.";

        let user_prompt = format!(
            "Optimize these SOAP operations for REST API design. For each operation, suggest:\n\
             - http_method: GET for queries/reads, POST for mutations/creates, PUT for updates, DELETE for deletions\n\
             - path: RESTful path (use kebab-case, plural nouns for collections)\n\n\
             Operations:\n{}\n\n\
             Respond with a JSON array of objects, each with: name, http_method, path",
            serde_json::to_string_pretty(&operations_summary)?
        );

        match llm.complete_json(&system_prompt, &user_prompt).await {
            Ok(suggestions) => {
                if let Some(arr) = suggestions.as_array() {
                    for suggestion in arr {
                        if let (Some(name), Some(method), Some(path)) = (
                            suggestion["name"].as_str(),
                            suggestion["http_method"].as_str(),
                            suggestion["path"].as_str(),
                        ) {
                            if let Some(op) = contract.operations.iter_mut().find(|o| o.name == name) {
                                op.http_method = method.to_string();
                                op.path = path.to_string();
                            }
                        }
                    }
                }
            }
            Err(e) => {
                // LLM 失败时降级到确定性映射结果，不影响核心流程
                tracing::warn!(error = %e, "LLM enhancement failed, using deterministic mapping");
            }
        }

        Ok(contract)
    }
}
```

- [ ] **Step 2: 更新 pipeline 支持可选 LLM**

在 `GenerationPipeline` 中添加 `run_wsdl_with_llm` 方法，当有 LLM 客户端时使用增强映射，否则回退到确定性映射。

- [ ] **Step 3: 更新 CLI 添加 LLM 配置**

添加可选的 `--llm-provider` 和读取 `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` 环境变量。

- [ ] **Step 4: 编写测试**

测试 LLM 失败时的降级行为（mock LlmClient 返回错误，验证仍然产出有效 contract）。

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(generator): add LLM-enhanced WSDL semantic mapper with graceful degradation"
```

---

### Task 4: 影子测试生成

**Files:**
- Create: `crates/generator/src/shadow_test.rs`

从 UnifiedContract 生成测试用例骨架。

- [ ] **Step 1: 实现 ShadowTestGenerator**

```rust
use crate::unified_contract::*;
use serde_json::{json, Value};

pub struct ShadowTestGenerator;

#[derive(Debug, Clone, serde::Serialize)]
pub struct ShadowTestCase {
    pub name: String,
    pub description: String,
    pub operation: String,
    pub method: String,
    pub path: String,
    pub request_body: Option<Value>,
    pub expected_status: u16,
}

impl ShadowTestGenerator {
    /// 为每个操作生成：正常请求、空 body、必填字段缺失
    pub fn generate(contract: &UnifiedContract) -> Vec<ShadowTestCase> {
        let mut tests = Vec::new();

        for op in &contract.operations {
            // 正常请求
            tests.push(ShadowTestCase {
                name: format!("{}_normal", op.name),
                description: format!("Normal request to {}", op.name),
                operation: op.name.clone(),
                method: op.http_method.clone(),
                path: op.path.clone(),
                request_body: op.input.as_ref().map(|m| Self::generate_sample_from_schema(&m.schema)),
                expected_status: 200,
            });

            // 空 body 请求（应返回 400 或正常处理）
            if op.input.is_some() {
                tests.push(ShadowTestCase {
                    name: format!("{}_empty_body", op.name),
                    description: format!("Empty body request to {}", op.name),
                    operation: op.name.clone(),
                    method: op.http_method.clone(),
                    path: op.path.clone(),
                    request_body: Some(json!({})),
                    expected_status: 200, // SOAP 通常不校验
                });
            }
        }

        tests
    }

    /// 根据 JSON Schema 生成样例数据
    fn generate_sample_from_schema(schema: &Value) -> Value {
        if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
            let mut obj = serde_json::Map::new();
            for (key, prop) in props {
                let val = match prop.get("type").and_then(|t| t.as_str()) {
                    Some("integer") => json!(1),
                    Some("number") => json!(1.0),
                    Some("boolean") => json!(true),
                    Some("array") => json!([]),
                    _ => json!("sample"),
                };
                obj.insert(key.clone(), val);
            }
            Value::Object(obj)
        } else {
            json!({})
        }
    }
}
```

- [ ] **Step 2: 编写测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::wsdl::{parser::WsdlParser, mapper::WsdlMapper};

    #[test]
    fn generates_tests_for_each_operation() {
        let wsdl = WsdlParser::parse(include_str!("../tests/fixtures/calculator.wsdl")).unwrap();
        let contract = WsdlMapper::map(&wsdl).unwrap();
        let tests = ShadowTestGenerator::generate(&contract);
        // 2 operations × 2 cases (normal + empty_body) = 4
        assert_eq!(tests.len(), 4);
    }

    #[test]
    fn normal_test_has_sample_body() {
        let wsdl = WsdlParser::parse(include_str!("../tests/fixtures/calculator.wsdl")).unwrap();
        let contract = WsdlMapper::map(&wsdl).unwrap();
        let tests = ShadowTestGenerator::generate(&contract);
        let normal = tests.iter().find(|t| t.name == "Add_normal").unwrap();
        let body = normal.request_body.as_ref().unwrap();
        assert!(body.get("a").is_some());
        assert!(body.get("b").is_some());
    }

    #[test]
    fn sample_from_schema_respects_types() {
        let schema = json!({
            "properties": {
                "count": {"type": "integer"},
                "name": {"type": "string"},
                "active": {"type": "boolean"}
            }
        });
        let sample = ShadowTestGenerator::generate_sample_from_schema(&schema);
        assert!(sample["count"].is_number());
        assert!(sample["name"].is_string());
        assert!(sample["active"].is_boolean());
    }
}
```

- [ ] **Step 3: 集成到 pipeline**

在 pipeline 的 Stage 4 位置调用 ShadowTestGenerator，将测试用例写入 Artifact。

- [ ] **Step 4: Commit**

```bash
git commit -am "feat(generator): add shadow test generation from UnifiedContract"
```

---

### Task 5: Agent 提示词生成

**Files:**
- Create: `crates/generator/src/agent_prompt.rs`

为每个 API 生成结构化 Prompt，AI Agent 可直接消费。

- [ ] **Step 1: 实现 AgentPromptGenerator**

```rust
use crate::unified_contract::*;

pub struct AgentPromptGenerator;

impl AgentPromptGenerator {
    pub fn generate(contract: &UnifiedContract) -> String {
        let mut prompt = String::new();
        prompt.push_str(&format!("# {} API\n\n", contract.service_name));
        prompt.push_str(&format!("{}\n\n", contract.description));
        prompt.push_str("## Available Operations\n\n");

        for op in &contract.operations {
            prompt.push_str(&format!("### {}\n", op.name));
            prompt.push_str(&format!("- **Method:** {}\n", op.http_method));
            prompt.push_str(&format!("- **Path:** {}\n", op.path));
            prompt.push_str(&format!("- **Description:** {}\n", op.description));

            if let Some(input) = &op.input {
                prompt.push_str(&format!("- **Request Body:** `{}`\n", serde_json::to_string(&input.schema).unwrap_or_default()));
            }
            if let Some(output) = &op.output {
                prompt.push_str(&format!("- **Response:** `{}`\n", serde_json::to_string(&output.schema).unwrap_or_default()));
            }
            prompt.push('\n');
        }

        prompt.push_str("## Usage Example\n\n");
        if let Some(first_op) = contract.operations.first() {
            let sample_body = first_op.input.as_ref()
                .map(|m| crate::shadow_test::ShadowTestGenerator::generate_sample_from_schema(&m.schema));
            prompt.push_str(&format!("```bash\ncurl -X {} {}{}", first_op.http_method, "{{base_url}}", first_op.path));
            if let Some(body) = sample_body {
                prompt.push_str(&format!(" \\\n  -H 'Content-Type: application/json' \\\n  -d '{}'\n", serde_json::to_string(&body).unwrap_or_default()));
            }
            prompt.push_str("```\n");
        }

        prompt
    }
}
```

- [ ] **Step 2: 编写测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::wsdl::{parser::WsdlParser, mapper::WsdlMapper};

    #[test]
    fn generates_prompt_with_all_operations() {
        let wsdl = WsdlParser::parse(include_str!("../tests/fixtures/calculator.wsdl")).unwrap();
        let contract = WsdlMapper::map(&wsdl).unwrap();
        let prompt = AgentPromptGenerator::generate(&contract);
        assert!(prompt.contains("CalculatorService"));
        assert!(prompt.contains("Add"));
        assert!(prompt.contains("GetHistory"));
        assert!(prompt.contains("curl"));
    }

    #[test]
    fn prompt_includes_request_schema() {
        let wsdl = WsdlParser::parse(include_str!("../tests/fixtures/calculator.wsdl")).unwrap();
        let contract = WsdlMapper::map(&wsdl).unwrap();
        let prompt = AgentPromptGenerator::generate(&contract);
        assert!(prompt.contains("Request Body"));
    }
}
```

- [ ] **Step 3: Commit**

```bash
git commit -am "feat(generator): add Agent prompt generation from UnifiedContract"
```

---

### Task 6: 端到端集成验证

**Files:**
- Create: `crates/platform-api/tests/e2e_soap_proxy_test.rs`

使用 wiremock 创建 mock SOAP 服务，验证完整链路：generate → load → proxy。

- [ ] **Step 1: 添加 wiremock dev-dependency**

在 platform-api Cargo.toml 添加 `wiremock = "0.6"` dev-dependency。

- [ ] **Step 2: 编写 E2E 测试**

```rust
#[tokio::test]
async fn e2e_wsdl_generate_and_proxy() {
    // 1. 启动 mock SOAP 服务
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("Content-Type", "text/xml; charset=utf-8"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_string(soap_response_xml()))
        .mount(&mock_server)
        .await;

    // 2. 修改 WSDL 中的 endpoint 指向 mock 服务
    let wsdl = sample_wsdl().replace(
        "http://example.com/calculator",
        &mock_server.uri(),
    );

    // 3. 运行 generate pipeline
    let repo = setup_repo().await;
    let project = repo.create_project(...).await.unwrap();
    let result = GenerationPipeline::run_wsdl(&repo, project.id, &wsdl).await.unwrap();
    assert_eq!(result.routes_count, 2);

    // 4. 加载路由到网关
    let router = Arc::new(DynamicRouter::new());
    let dispatchers = Arc::new(DashMap::new());
    RouteLoader::load(&repo, &router, &dispatchers).await.unwrap();

    // 5. 构建带网关的 app 并发送请求
    let app = build_test_app_with_gateway(pool, router, dispatchers);
    let server = TestServer::new(app).unwrap();

    let resp = server.post("/gw/api/v1/calculator/add")
        .json(&json!({"a": 5, "b": 3}))
        .await;

    resp.assert_status(StatusCode::OK);
    let body: Value = resp.json();
    assert_eq!(body["result"], "8");  // mock 返回的值
}
```

- [ ] **Step 3: 运行全量测试**

Run: `DATABASE_URL=... cargo test --workspace`
Expected: 所有测试通过

- [ ] **Step 4: Commit**

```bash
git commit -am "test: add E2E test for WSDL→generate→gateway proxy with mock SOAP server"
```

---

## Summary

| Task | 组件 | 产出 |
|------|------|------|
| 1 | Route Loader | 启动时从 DB 加载路由 → 创建适配器 → 注册 DynamicRouter |
| 2 | LLM 适配层 | LlmClient trait + Claude/OpenAI 客户端 + 3 测试 |
| 3 | LLM Mapper | LLM 增强语义映射 + 降级机制 + 测试 |
| 4 | Shadow Tests | 影子测试生成器 + 3 测试 |
| 5 | Agent Prompt | 结构化提示词生成 + 2 测试 |
| 6 | E2E | mock SOAP → generate → load → proxy 全链路验证 |

**Phase 1c 验收标准：**
- 网关启动时自动加载已生成的路由
- 通过 `/gw/...` 端点可实际代理 SOAP 请求（JSON in → SOAP XML → backend → SOAP XML → JSON out）
- LLM 适配层支持 Claude/OpenAI，失败时优雅降级
- 影子测试和 Agent 提示词可从 UnifiedContract 自动生成
- E2E 测试使用 mock SOAP 服务验证完整链路

**Phase 1 整体验收标准达成：** 能将一个真实 WSDL 自动转为可用的 REST API。
