# Phase 3: 沙箱测试平台 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 构建沙箱测试平台，让下游系统在不依赖真实后端的情况下完成 API 联调。提供三层递进能力：Mock（Schema 驱动假数据）、Replay（录制回放）、Proxy（真实后端代理 + 隔离）。

**Architecture:** 新建 `sandbox` crate 封装所有沙箱逻辑。沙箱通过独立路径 `/sandbox/*` 提供服务，通过请求头 `X-Sandbox-Mode` 和 `X-Sandbox-Tenant` 区分模式和租户。Mock 层从元数据中的 response_schema 自动生成假数据。Replay 层从 recorded_interactions 表匹配回放。Proxy 层复用网关的 BackendDispatcher 但注入租户标记。

**Tech Stack:** rand (假数据生成), 已有 metadata + gateway crate

**Spec:** `docs/superpowers/specs/2026-03-21-api-anything-platform-design.md` §6

---

## File Structure

```
crates/sandbox/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── mock_layer.rs               # Schema → 假数据生成
    ├── replay_layer.rs             # 录制匹配 + 回放
    ├── proxy_layer.rs              # 真实后端代理 + 租户隔离
    ├── recorder.rs                 # 交互录制引擎
    ├── handler.rs                  # Sandbox Axum handler + 模式路由
    └── session.rs                  # 沙箱会话管理
```

同时修改:
- `Cargo.toml` (workspace) — 添加 sandbox 成员
- `crates/metadata/src/repo.rs` — 扩展沙箱会话和录制交互的 CRUD
- `crates/platform-api/` — 挂载 sandbox handler + 会话管理 API

---

### Task 1: Sandbox Crate 脚手架 + 会话管理

**Files:**
- Create: `crates/sandbox/Cargo.toml`
- Create: `crates/sandbox/src/lib.rs`
- Create: `crates/sandbox/src/session.rs`
- Modify: `Cargo.toml` (workspace)
- Modify: `crates/metadata/src/repo.rs` — 会话 CRUD
- Modify: `crates/metadata/src/pg.rs` — 会话 CRUD 实现
- Modify: `crates/platform-api/` — 会话管理 API 端点

沙箱会话代表一个下游团队的测试环境，绑定 project + tenant + mode。

MetadataRepo 扩展：
```rust
async fn create_sandbox_session(&self, project_id: Uuid, tenant_id: &str, mode: SandboxMode, config: &Value, expires_at: DateTime<Utc>) -> Result<SandboxSession, AppError>;
async fn get_sandbox_session(&self, id: Uuid) -> Result<SandboxSession, AppError>;
async fn list_sandbox_sessions(&self, project_id: Uuid) -> Result<Vec<SandboxSession>, AppError>;
async fn delete_sandbox_session(&self, id: Uuid) -> Result<(), AppError>;
```

Platform API 端点：
- `POST /api/v1/projects/{project_id}/sandbox-sessions` — 创建会话
- `GET /api/v1/projects/{project_id}/sandbox-sessions` — 列出会话
- `DELETE /api/v1/sandbox-sessions/{id}` — 删除会话

测试：会话 CRUD 3 测试

Commit: `feat(sandbox): add sandbox crate scaffold and session management`

---

### Task 2: Mock Layer — Schema 驱动假数据生成

**Files:**
- Create: `crates/sandbox/src/mock_layer.rs`

从路由的 response_schema (JSON Schema) 自动生成符合 schema 的假数据。

**三种生成策略：**

1. **Smart Mock** — 根据字段名语义生成合理数据：
   - `email` → `"user@example.com"`
   - `phone` → `"+86-13800001234"`
   - `name`/`username` → `"John Doe"`
   - `id` → UUID 字符串
   - `amount`/`price` → `99.50`
   - `date`/`created_at` → ISO 8601 日期
   - `status` → `"active"`
   - `count`/`total` → 随机整数

2. **Schema Mock** — 严格按 JSON Schema 类型生成随机值

3. **Fixed Mock** — 用户自定义的固定响应（从 session config 中读取）

```rust
pub struct MockLayer;

impl MockLayer {
    /// 根据路由的 response_schema 生成 mock 响应
    pub fn generate(schema: &Value, config: &Value) -> Value {
        // 如果 config 中有 fixed_response，直接返回
        if let Some(fixed) = config.get("fixed_response") {
            return fixed.clone();
        }
        Self::generate_from_schema(schema)
    }

    fn generate_from_schema(schema: &Value) -> Value { ... }
}
```

测试 (4)：
```rust
#[test] fn generates_object_from_schema() { ... }
#[test] fn smart_mock_email_field() { ... }
#[test] fn respects_enum_values() { ... }
#[test] fn returns_fixed_response_when_configured() { ... }
```

Commit: `feat(sandbox): add mock layer with schema-driven data generation`

---

### Task 3: Replay Layer — 录制引擎 + 匹配回放

**Files:**
- Create: `crates/sandbox/src/recorder.rs`
- Create: `crates/sandbox/src/replay_layer.rs`
- Modify: `crates/metadata/src/repo.rs` — 录制交互 CRUD

录制引擎在 Proxy 模式下自动记录请求-响应对到 recorded_interactions 表。回放引擎根据匹配策略查找最相似的已录制交互。

MetadataRepo 扩展：
```rust
async fn record_interaction(&self, session_id: Uuid, route_id: Uuid, request: &Value, response: &Value, duration_ms: i32) -> Result<RecordedInteraction, AppError>;
async fn find_matching_interaction(&self, session_id: Uuid, route_id: Uuid, request: &Value) -> Result<Option<RecordedInteraction>, AppError>;
async fn list_recorded_interactions(&self, session_id: Uuid) -> Result<Vec<RecordedInteraction>, AppError>;
```

**匹配策略：**
- 精确匹配：route_id + request body 完全一致
- 模糊匹配：route_id 相同，忽略 timestamp/random 字段，按业务键匹配
- 无匹配：返回 404 + 最相似录制的提示

```rust
pub struct ReplayLayer;

impl ReplayLayer {
    pub async fn replay(
        repo: &impl MetadataRepo,
        session_id: Uuid,
        route_id: Uuid,
        request: &Value,
    ) -> Result<Value, AppError> {
        // Try exact match first, then fuzzy
        if let Some(interaction) = repo.find_matching_interaction(session_id, route_id, request).await? {
            return Ok(interaction.response);
        }
        Err(AppError::NotFound("No matching recorded interaction".into()))
    }
}
```

测试 (3)：精确匹配、无匹配返回 404、录制后可回放

Commit: `feat(sandbox): add replay layer with recording engine and matching`

---

### Task 4: Proxy Layer — 真实后端代理 + 租户隔离

**Files:**
- Create: `crates/sandbox/src/proxy_layer.rs`

Proxy 模式将请求转发到真实后端（复用网关的 BackendDispatcher），同时：
1. 注入 `X-Sandbox-Tenant` 到后端请求头
2. 自动录制交互（调用 recorder）
3. 支持只读模式（仅允许 GET）

```rust
pub struct ProxyLayer;

impl ProxyLayer {
    pub async fn proxy(
        dispatcher: &BackendDispatcher,
        recorder: &Recorder,
        session: &SandboxSession,
        route_id: Uuid,
        mut gateway_req: GatewayRequest,
    ) -> Result<GatewayResponse, AppError> {
        // Read-only check
        if session.config.get("read_only") == Some(&json!(true)) && gateway_req.method != Method::GET {
            return Err(AppError::BadRequest("Sandbox is read-only, only GET allowed".into()));
        }

        // Inject tenant header
        gateway_req.headers.insert(
            "X-Sandbox-Tenant",
            session.tenant_id.parse().unwrap(),
        );

        // Forward to real backend
        let resp = dispatcher.dispatch(gateway_req.clone()).await?;

        // Record interaction
        recorder.record(session.id, route_id, &gateway_req, &resp).await?;

        Ok(resp)
    }
}
```

测试 (2)：只读模式阻止 POST、租户头注入

Commit: `feat(sandbox): add proxy layer with tenant isolation and auto-recording`

---

### Task 5: Sandbox Handler — 模式路由

**Files:**
- Create: `crates/sandbox/src/handler.rs`
- Modify: `crates/platform-api/src/lib.rs` — 挂载 /sandbox/*
- Modify: `crates/platform-api/src/state.rs` — 可能需要扩展

统一入口 `/sandbox/*`，通过 `X-Sandbox-Mode` 头分发到 Mock/Replay/Proxy 层。

```rust
pub async fn sandbox_handler(
    State(state): State<AppState>,
    req: Request,
) -> Result<impl IntoResponse, AppError> {
    let mode = req.headers().get("X-Sandbox-Mode")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("mock");

    let session_id = req.headers().get("X-Sandbox-Session")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok());

    let path = req.uri().path().strip_prefix("/sandbox").unwrap_or(req.uri().path());

    // Route matching (same as gateway)
    let (route_id, path_params) = state.router.match_route(&method, path)
        .ok_or_else(|| AppError::NotFound(...))?;

    match mode {
        "mock" => {
            // Get route's response_schema from metadata
            // Generate mock response
        }
        "replay" => {
            // Need session_id
            // Find matching recorded interaction
        }
        "proxy" => {
            // Need session_id
            // Forward to real backend via dispatcher
        }
        _ => Err(AppError::BadRequest("Invalid sandbox mode"))
    }
}
```

在 platform-api 中挂载：
```rust
.route("/sandbox/{*rest}", any(sandbox::handler::sandbox_handler))
```

测试 (3)：
- Mock 模式返回合法 JSON
- 未知模式返回 400
- 无匹配路由返回 404

Commit: `feat(sandbox): add sandbox handler with mode routing`

---

### Task 6: 端到端集成测试

**Files:**
- Create: `crates/platform-api/tests/e2e_sandbox_test.rs`

使用已有的 calculator WSDL 生成路由，然后测试三层沙箱：

```rust
#[tokio::test]
async fn sandbox_mock_returns_data_matching_schema() {
    // Generate routes from WSDL → load
    // POST /sandbox/api/v1/calculator/add with X-Sandbox-Mode: mock
    // Verify response has correct schema structure
}

#[tokio::test]
async fn sandbox_replay_returns_404_without_recordings() {
    // X-Sandbox-Mode: replay without prior recordings
    // Should return 404
}

#[tokio::test]
async fn sandbox_session_crud() {
    // Create session → list → delete
    // Verify lifecycle
}
```

Commit: `test: add E2E sandbox tests for mock, replay, and session management`

---

## Summary

| Task | 组件 | 产出 |
|------|------|------|
| 1 | Sandbox Crate + Session CRUD | 脚手架 + 会话管理 API + 3 测试 |
| 2 | Mock Layer | Schema → 假数据 + Smart Mock + 4 测试 |
| 3 | Replay Layer | 录制引擎 + 匹配回放 + 3 测试 |
| 4 | Proxy Layer | 租户隔离 + 自动录制 + 2 测试 |
| 5 | Sandbox Handler | /sandbox/* 模式路由 + 3 测试 |
| 6 | E2E | 全链路沙箱验证 + 3 测试 |

**Phase 3 验收标准：** 下游系统可通过 `/sandbox/*` 端点，使用 `X-Sandbox-Mode: mock` 获取符合 Schema 的假数据进行联调，使用 `replay` 回放录制数据，使用 `proxy` 代理到真实后端。沙箱会话可通过 API 创建和管理。
