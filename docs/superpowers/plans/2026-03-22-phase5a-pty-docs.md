# Phase 5a: PTY 适配器 + API 文档服务 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现交互式 PTY 会话包装（Expect 状态机模式）和 API 文档在线服务（OpenAPI JSON 端点 + Swagger UI），完成所有 Rust 后端功能。

**Architecture:** PTY 适配器基于 tokio::process 的 stdin/stdout pipe 实现简化版 Expect 状态机（发送命令→等待提示符→读取输出）。API 文档服务从元数据动态生成 OpenAPI spec 并通过端点提供，同时内嵌 Swagger UI 静态页面。

**Tech Stack:** tokio::process (PTY stdin/stdout), 已有 generator OpenAPI 模块

**Spec:** `docs/superpowers/specs/2026-03-21-api-anything-platform-design.md` §4.3, §5, §8

---

## File Structure

```
crates/gateway/src/adapters/
    └── pty_expect.rs               # PTY ProtocolAdapter (Expect 状态机)

crates/platform-api/src/routes/
    ├── docs.rs                     # OpenAPI JSON + Swagger UI 端点
    └── ...
```

---

### Task 1: PTY Expect 适配器

**Files:**
- Create: `crates/gateway/src/adapters/pty_expect.rs`
- Modify: `crates/gateway/src/adapters/mod.rs`
- Modify: `crates/gateway/src/loader.rs` — PTY 分支

PTY 适配器用于交互式程序（如数据库客户端、网络设备 CLI），通过 stdin 发送命令，从 stdout 读取响应，使用提示符检测命令完成。

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyConfig {
    pub program: String,
    pub args: Vec<String>,
    pub prompt_pattern: String,    // 正则匹配提示符，如 r">\s*$" 或 r"\$\s*$"
    pub command_template: String,  // 要执行的命令模板
    pub output_format: OutputFormat,
    pub init_commands: Vec<String>, // 启动后的初始化命令序列
    pub timeout_ms: u64,
}
```

execute 流程：
1. spawn 进程（tokio::process::Command with stdin/stdout piped）
2. 等待初始提示符出现
3. 执行 init_commands（逐条发送，等待提示符）
4. 发送实际命令
5. 等待提示符 → 收集输出
6. kill 进程

由于 PTY 是最重最慢的后端类型（默认并发 3，超时 300s），保护策略最严格。

测试 (3)：
- transform_request 参数替换
- transform_response 输出解析
- execute 使用 `cat` 或 `bash -c` 做简单验证

Commit: `feat(gateway): add PTY expect-style protocol adapter`

---

### Task 2: OpenAPI 文档服务端点

**Files:**
- Create: `crates/platform-api/src/routes/docs.rs`
- Modify: `crates/platform-api/src/routes/mod.rs`
- Modify: `crates/platform-api/src/lib.rs`

动态生成 OpenAPI spec：从元数据加载所有活跃路由，用 generator 的 OpenApiGenerator 生成规范。

端点：
- `GET /api/v1/docs/openapi.json` — 返回动态生成的 OpenAPI 3.0 JSON
- `GET /api/v1/docs` — 返回嵌入 Swagger UI 的 HTML 页面（指向 openapi.json）

```rust
pub async fn openapi_json(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let routes = state.repo.list_active_routes_with_bindings().await?;
    // 从路由构建简化的 UnifiedContract 用于生成 OpenAPI
    // 或直接从路由数据构建 OpenAPI paths
    let spec = build_openapi_from_routes(&routes);
    Ok(Json(spec))
}

pub async fn swagger_ui() -> impl IntoResponse {
    let html = r#"<!DOCTYPE html>
<html><head><title>API-Anything Docs</title>
<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui.css">
</head><body>
<div id="swagger-ui"></div>
<script src="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
<script>SwaggerUIBundle({url:'/api/v1/docs/openapi.json',dom_id:'#swagger-ui'})</script>
</body></html>"#;
    axum::response::Html(html)
}
```

测试 (3)：
- openapi.json 返回有效 JSON
- openapi.json 包含已注册路由的 paths
- swagger_ui 返回 HTML

Commit: `feat(platform-api): add OpenAPI JSON endpoint and Swagger UI`

---

### Task 3: Agent 提示词服务端点 + 完整性测试

**Files:**
- Modify: `crates/platform-api/src/routes/docs.rs`

端点：
- `GET /api/v1/docs/agent-prompt` — 返回 Agent 可消费的结构化提示词
- `GET /api/v1/projects/{id}/openapi.json` — 返回特定项目的 OpenAPI spec

测试 (2)：
- agent-prompt 返回包含操作描述的 Markdown
- 项目级 OpenAPI 只包含该项目的路由

Commit: `feat(platform-api): add agent prompt and project-scoped OpenAPI endpoints`

---

## Summary

| Task | 组件 | 产出 |
|------|------|------|
| 1 | PTY Adapter | Expect 状态机 + 3 测试 |
| 2 | OpenAPI 服务 | JSON 端点 + Swagger UI + 3 测试 |
| 3 | Agent Prompt + 项目级 API | 结构化提示词 + 项目 OpenAPI + 2 测试 |

**Phase 5a 验收标准：** PTY 适配器可通过 stdin/stdout pipe 与交互式程序通信。OpenAPI spec 可从 `/api/v1/docs/openapi.json` 获取，Swagger UI 可在 `/api/v1/docs` 访问。Agent prompt 可从端点获取。
