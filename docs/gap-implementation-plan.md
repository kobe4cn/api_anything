# API-Anything 差距项补全规划

> 对照设计规格的 16 个未实现项，逐一分析实现方案、影响范围和风险

---

## 总览

| # | 差距项 | 优先级 | 工作量 | 影响面 | 风险 |
|---|--------|--------|--------|--------|------|
| 1 | TLS 终结 (rustls) | P1 | 1天 | main.rs + config | 低 |
| 2 | Auth Guard (JWT) | P1 | 2天 | 新增中间件 + 所有请求链路 | 中 |
| 3 | 路由热加载定时轮询 | P1 | 0.5天 | main.rs + gateway | 低 |
| 4 | source_config 加密存储 | P2 | 1天 | common + metadata | 低 |
| 5 | 数据染色 | P3 | 0.5天 | sandbox proxy_layer | 低 |
| 6 | WebSocket 实时更新 | P3 | 2天 | platform-api + web | 中 |
| 7 | Artifact 产物管理 | P3 | 1天 | metadata + platform-api | 低 |
| 8 | 修改 payload 后重推 | P3 | 0.5天 | compensation + platform-api | 低 |
| 9 | OTel 自定义指标 | P2 | 1.5天 | gateway 全模块 | 中 |
| 10 | K8s 部署清单 + HPA | P2 | 1天 | 新增 YAML 文件 | 低 |
| 11 | Rust 服务 Dockerfile | P2 | 0.5天 | 新增文件 | 低 |
| 12 | Webhook 推送格式配置 | P3 | 1天 | compensation push_dispatcher | 低 |
| 13 | OData 协议支持 | P4 | 3天 | generator + gateway | 中 |
| 14 | Replay 无匹配提示 | P4 | 0.5天 | sandbox replay_layer | 低 |
| 15 | 推送历史和成功率 | P3 | 1天 | compensation + metadata | 低 |
| 16 | OTel Tail Sampling | P3 | 0.5天 | docker 配置 | 低 |

---

## 详细设计

### 1. TLS 终结 (rustls) — P1

**当前状态：** `TcpListener::bind` 直接监听 HTTP 明文。

**实现方案：**
```rust
// main.rs 变更
// 新增配置项
pub struct AppConfig {
    // ...existing...
    pub tls_cert_path: Option<String>,  // TLS_CERT_PATH
    pub tls_key_path: Option<String>,   // TLS_KEY_PATH
}

// 启动时判断
if let (Some(cert), Some(key)) = (&config.tls_cert_path, &config.tls_key_path) {
    let tls_config = RustlsConfig::from_pem_file(cert, key).await?;
    axum_server::bind_rustls(addr, tls_config)
        .serve(app.into_make_service()).await?;
} else {
    // 保持现有 HTTP 启动逻辑（开发模式）
    axum::serve(listener, app).await?;
}
```

**依赖新增：** `axum-server = { version = "0.7", features = ["tls-rustls"] }`

**影响范围：**
- `crates/common/src/config.rs` — 新增 2 个可选配置项
- `crates/platform-api/src/main.rs` — 启动逻辑分支
- `Cargo.toml` — 新增 axum-server 依赖

**对现有功能的影响：** **零影响**。TLS 是可选的，不配置证书时行为完全不变。所有现有测试使用 HTTP，不受影响。

**风险：** 低。axum-server 是成熟库，rustls 无 C 依赖。

---

### 2. Auth Guard (JWT 验证 + 凭证翻译) — P1

**当前状态：** 无任何认证机制，所有端点裸露。

**实现方案：**

新建 `crates/platform-api/src/middleware/auth_guard.rs`：

```rust
pub struct AuthConfig {
    pub enabled: bool,
    pub jwt_secret: String,         // JWT_SECRET
    pub jwt_issuer: Option<String>, // JWT_ISSUER
    pub skip_paths: Vec<String>,    // 跳过认证的路径（/health, /api/v1/docs）
}

pub async fn auth_middleware(
    State(config): State<AuthConfig>,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let path = req.uri().path();

    // 跳过不需要认证的路径
    if config.skip_paths.iter().any(|p| path.starts_with(p)) || !config.enabled {
        return Ok(next.run(req).await);
    }

    // 提取 Bearer token
    let token = req.headers().get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(AppError::Unauthorized)?;

    // 验证 JWT
    let claims = decode_jwt(token, &config.jwt_secret)?;

    // 将用户信息注入请求扩展
    req.extensions_mut().insert(claims);

    Ok(next.run(req).await)
}
```

凭证翻译层（在 gateway handler 中）：
```rust
// 根据 BackendBinding.auth_mapping 翻译凭证
// SOAP → 注入 WS-Security Header
// CLI → 映射到 Linux 用户
// SSH → 从配置获取 SSH Key
// 这部分通过 adapter 的 transform_request 中读取 auth_mapping 实现
```

**依赖新增：** `jsonwebtoken = "9"`

**影响范围：**
- `crates/common/src/config.rs` — 新增 auth 配置项
- `crates/common/src/error.rs` — 新增 `AppError::Unauthorized` (401)
- `crates/platform-api/src/middleware/auth_guard.rs` — 新建
- `crates/platform-api/src/lib.rs` — 添加中间件层

**对现有功能的影响：** **条件性影响**。
- `AUTH_ENABLED=false`（默认）时：完全不影响，所有现有测试通过
- `AUTH_ENABLED=true` 时：所有请求需要 JWT，现有集成测试需添加 token
- 建议：测试环境默认关闭 auth，生产环境开启

**风险：** 中。中间件位于所有请求链路上，错误实现会导致全站不可用。需要充分测试 skip_paths 白名单。

---

### 3. 路由热加载定时轮询 — P1

**当前状态：** 启动时 `RouteLoader::load` 加载一次，运行中不刷新。

**实现方案：**

在 `main.rs` 中启动后台轮询任务：

```rust
// 路由热加载轮询（每 5 秒检查路由版本）
let reload_repo = repo.clone();
let reload_router = state.router.clone();
let reload_dispatchers = state.dispatchers.clone();
tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    let mut last_count = 0u64;
    loop {
        interval.tick().await;
        // 轻量检查：只查路由数量或版本号
        match reload_repo.list_active_routes_with_bindings().await {
            Ok(routes) if routes.len() as u64 != last_count => {
                // 路由表有变化，重新加载
                match RouteLoader::load(reload_repo.as_ref(), &reload_router, &reload_dispatchers).await {
                    Ok(loaded) => {
                        last_count = loaded as u64;
                        tracing::info!(routes = loaded, "Routes hot-reloaded");
                    }
                    Err(e) => tracing::error!(error = %e, "Route reload failed"),
                }
            }
            Ok(routes) => { last_count = routes.len() as u64; }
            Err(e) => tracing::error!(error = %e, "Route check failed"),
        }
    }
});
```

**影响范围：**
- `crates/platform-api/src/main.rs` — 添加 ~20 行

**对现有功能的影响：** **零影响**。纯新增后台任务，不修改任何现有代码路径。DynamicRouter 的 RCU 机制已经支持原子替换，轮询只是触发机制。

**风险：** 低。最坏情况是轮询查询增加 DB 负载（每 5 秒一次轻量查询）。

---

### 4. source_config 加密存储 — P2

**当前状态：** `endpoint_config` JSONB 明文存储，含 SSH 密码、SOAP 凭证等敏感信息。

**实现方案：**

新建 `crates/common/src/crypto.rs`：

```rust
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};

pub struct Encryptor {
    key: LessSafeKey,
}

impl Encryptor {
    pub fn from_env() -> Option<Self> {
        let key_hex = std::env::var("ENCRYPTION_KEY").ok()?;
        // 32 字节 AES-256 密钥
        let key_bytes = hex::decode(&key_hex).ok()?;
        let unbound = UnboundKey::new(&AES_256_GCM, &key_bytes).ok()?;
        Some(Self { key: LessSafeKey::new(unbound) })
    }

    pub fn encrypt(&self, plaintext: &str) -> Result<String, anyhow::Error> { ... }
    pub fn decrypt(&self, ciphertext: &str) -> Result<String, anyhow::Error> { ... }
}
```

在 MetadataRepo 的 `create_backend_binding` 和 `get_*` 方法中透明加解密 `endpoint_config`。

**影响范围：**
- `crates/common/src/crypto.rs` — 新建
- `crates/common/src/config.rs` — 新增 `ENCRYPTION_KEY`
- `crates/metadata/src/pg.rs` — 写入时加密、读取时解密
- `Cargo.toml` — 新增 `ring`, `hex`

**对现有功能的影响：** **需谨慎处理**。
- 不配置 `ENCRYPTION_KEY` 时：保持明文存储（向后兼容）
- 配置后：新写入的数据加密，需要处理已有明文数据的迁移
- 现有测试不受影响（不配置 key 即为明文模式）

**风险：** 中。密钥管理是安全关键环节，丢失密钥则数据不可恢复。

---

### 5. 数据染色 (_sandbox: true) — P3

**当前状态：** Proxy 模式注入 `X-Sandbox-Tenant` 头，但未在请求/响应中标记 `_sandbox`。

**实现方案：**

在 `crates/sandbox/src/proxy_layer.rs` 的 `proxy()` 方法中：

```rust
// 在 gateway_req.body 中注入 _sandbox 标记
if let Some(body) = &mut gateway_req.body {
    if let Some(obj) = body.as_object_mut() {
        obj.insert("_sandbox".to_string(), json!(true));
        obj.insert("_sandbox_tenant".to_string(), json!(session.tenant_id));
    }
}
```

**影响范围：**
- `crates/sandbox/src/proxy_layer.rs` — 修改 ~5 行

**对现有功能的影响：** **微小影响**。
- Proxy 模式的请求 body 会多出 `_sandbox` 字段
- 后端系统需要能容忍额外字段（大多数 SOAP/REST 服务会忽略）
- 现有 proxy 测试的断言可能需要调整

**风险：** 低。但需要确认后端系统对额外字段的容忍度。

---

### 6. WebSocket 实时更新 — P3

**当前状态：** 前端通过手动刷新获取数据更新。

**实现方案：**

后端：在 platform-api 添加 WebSocket 端点 `/ws`：

```rust
// routes/ws.rs
pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_ws(socket, state))
}

async fn handle_ws(mut socket: WebSocket, state: AppState) {
    // 订阅事件总线
    // 当有 RouteUpdated / DeliveryFailed / DeadLetter 等事件时
    // 将事件序列化为 JSON 推送给客户端
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    loop {
        interval.tick().await;
        // 从 events 表查询最新事件
        // 发送给 WebSocket 客户端
    }
}
```

前端：在 Layout.tsx 中建立 WebSocket 连接，收到事件时触发对应页面刷新。

**影响范围：**
- `crates/platform-api/src/routes/ws.rs` — 新建
- `crates/platform-api/src/lib.rs` — 新增路由
- `web/src/components/Layout.tsx` — 添加 WS 连接
- `web/src/pages/*.tsx` — 添加自动刷新逻辑

**对现有功能的影响：** **零影响**。WebSocket 是独立端点，不影响现有 REST API。前端改动是增量的，不改变现有页面结构。

**风险：** 中。WebSocket 连接管理需要处理断线重连、并发控制等。

---

### 7. Artifact 产物管理 — P3

**当前状态：** `artifacts` 表存在但无 CRUD API，生成管道不写入 artifact 记录。

**实现方案：**

1. MetadataRepo 扩展：`create_artifact`, `list_artifacts`, `get_artifact`
2. Pipeline 修改：生成完成后写入 artifact 记录（openapi_json, test_suite, agent_prompt）
3. API 端点：`GET /api/v1/contracts/{id}/artifacts`

**影响范围：**
- `crates/metadata/src/repo.rs` + `pg.rs` — 新增 3 个方法
- `crates/generator/src/pipeline.rs` — 生成后写入 artifact
- `crates/platform-api/src/routes/` — 新增端点

**对现有功能的影响：** **低影响**。Pipeline 新增写入步骤，但失败不影响核心生成流程（catch 错误即可）。

**风险：** 低。

---

### 8. 修改 payload 后重推 — P3

**当前状态：** 死信可重推但使用原始 payload，不能修改。

**实现方案：**

新增 API 端点：
```
PUT /api/v1/compensation/delivery-records/{id}/payload
Body: { "request_payload": { ... } }
```

实现：更新 `delivery_records` 表的 `request_payload` 字段，然后触发重推。

**影响范围：**
- `crates/metadata/src/repo.rs` — 新增 `update_delivery_payload`
- `crates/platform-api/src/routes/compensation.rs` — 新增端点
- `web/src/pages/CompensationManager.tsx` — 添加编辑 payload 按钮

**对现有功能的影响：** **零影响**。纯新增功能。

**风险：** 低。

---

### 9. OTel 自定义指标 — P2

**当前状态：** 仅有 TraceLayer 的自动 trace span，无自定义 counter/histogram。

**实现方案：**

在 gateway 的关键路径上添加 OTel metrics：

```rust
use opentelemetry::metrics::{Counter, Histogram, Meter};

pub struct GatewayMetrics {
    pub request_total: Counter<u64>,
    pub request_duration: Histogram<f64>,
    pub backend_duration: Histogram<f64>,
    pub circuit_breaker_state: /* Gauge */,
    pub delivery_retry_total: Counter<u64>,
    pub dead_letter_total: Counter<u64>,
}

impl GatewayMetrics {
    pub fn new(meter: &Meter) -> Self {
        Self {
            request_total: meter.u64_counter("gateway.request.total").build(),
            request_duration: meter.f64_histogram("gateway.request.duration").build(),
            // ...
        }
    }
}
```

在 `gateway_handler` 和 `BackendDispatcher` 中记录指标。

**影响范围：**
- `crates/gateway/src/metrics.rs` — 新建
- `crates/gateway/src/dispatcher.rs` — 添加 metrics 记录
- `crates/platform-api/src/routes/gateway.rs` — 添加 request metrics
- `crates/platform-api/src/state.rs` — AppState 新增 metrics
- `crates/platform-api/src/main.rs` — 初始化 MeterProvider

**对现有功能的影响：** **微小影响**。在请求链路上添加了指标记录调用，增加极小的延迟（纳秒级）。需要修改 dispatcher 和 handler 的函数签名以传入 metrics。

**风险：** 中。opentelemetry metrics API 版本与 tracing 版本需要兼容。

---

### 10. K8s 部署清单 + HPA — P2

**当前状态：** 无 Kubernetes 配置文件。

**实现方案：**

创建 `deploy/k8s/` 目录：

```yaml
# deploy/k8s/namespace.yaml
# deploy/k8s/configmap.yaml (环境变量)
# deploy/k8s/secret.yaml (敏感配置模板)
# deploy/k8s/deployment.yaml (platform-api Deployment)
# deploy/k8s/service.yaml (ClusterIP Service)
# deploy/k8s/hpa.yaml (HorizontalPodAutoscaler)
# deploy/k8s/ingress.yaml (Ingress/TLS)
```

HPA 配置：
```yaml
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: api-anything-gateway
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: api-anything
  minReplicas: 2
  maxReplicas: 20
  metrics:
  - type: Resource
    resource:
      name: cpu
      target:
        type: Utilization
        averageUtilization: 70
```

**影响范围：** 纯新增文件，不修改任何现有代码。

**对现有功能的影响：** **零影响**。

**风险：** 低。但需要实际 K8s 集群验证。

---

### 11. Rust 服务 Dockerfile (musl scratch) — P2

**当前状态：** 仅有 `web/Dockerfile`（前端），无 Rust 服务容器化。

**实现方案：**

创建 `Dockerfile`：

```dockerfile
# Stage 1: Build
FROM rust:1.82-alpine AS builder
RUN apk add --no-cache musl-dev openssl-dev openssl-libs-static cmake make g++
WORKDIR /app
COPY . .
RUN cargo build --release --target x86_64-unknown-linux-musl -p api-anything-platform-api

# Stage 2: Runtime (< 20MB)
FROM scratch
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/api-anything-platform-api /
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
EXPOSE 8080
ENTRYPOINT ["/api-anything-platform-api"]
```

**影响范围：** 新增 `Dockerfile` 和 `.dockerignore`。

**对现有功能的影响：** **零影响**。

**风险：** 中。musl 静态编译可能遇到 C 依赖问题（rdkafka 需要 cmake，russh 需要 openssl）。可能需要条件编译排除某些 feature。

---

### 12. Webhook 推送格式配置 — P3

**当前状态：** Push Dispatcher 仅发送 JSON 格式。

**实现方案：**

在 `WebhookSubscription` 模型中添加 `format` 字段：

```rust
pub enum PushFormat {
    Json,                          // 默认
    Xml,                           // XML 格式
    Template { template: String }, // 自定义 Handlebars 模板
}
```

在 `push_dispatcher.rs` 中根据 format 序列化：
```rust
let body = match &sub.format {
    PushFormat::Json => serde_json::to_string(&payload)?,
    PushFormat::Xml => json_to_xml(&payload)?,
    PushFormat::Template { template } => render_template(template, &payload)?,
};
```

**影响范围：**
- `crates/common/src/models.rs` — WebhookSubscription 新增 format 字段
- `crates/compensation/src/push_dispatcher.rs` — 格式化逻辑
- DB migration — ALTER TABLE webhook_subscriptions ADD COLUMN format

**对现有功能的影响：** **低影响**。新增字段默认 JSON，现有订阅不受影响。

**风险：** 低。

---

### 13. OData 协议支持 — P4

**当前状态：** 规格中提到但设计时明确排在最低优先级。

**实现方案：**

新建 `crates/generator/src/odata/`:
- `parser.rs` — 解析 OData $metadata XML
- `mapper.rs` — EntityType/EntitySet → UnifiedContract

新建 `crates/gateway/src/adapters/odata.rs`:
- ODataAdapter — HTTP GET/POST 转发 + OData query 参数映射

**影响范围：** 纯新增文件 + RouteLoader 新增分支。

**对现有功能的影响：** **零影响**。新增协议不修改现有适配器。

**风险：** 中。OData 协议复杂度高（$filter, $expand, $select 等查询语法）。

---

### 14. Replay 无匹配提示 — P4

**当前状态：** 无匹配时返回 404，无最相似录制的提示。

**实现方案：**

在 `find_matching_interaction` 的模糊匹配逻辑中，如果无精确匹配，返回相似度最高的录制的摘要：

```rust
// 修改返回类型
pub enum MatchResult {
    Exact(RecordedInteraction),
    Closest { interaction: RecordedInteraction, similarity: f64 },
    None,
}
```

404 响应中包含提示：
```json
{
    "status": 404,
    "detail": "No matching recording found",
    "closest_match": {
        "recorded_at": "2024-01-15T10:30:00Z",
        "similarity": 0.75,
        "request_preview": { "a": 10, "b": 20 }
    }
}
```

**影响范围：**
- `crates/metadata/src/pg.rs` — 修改查询逻辑
- `crates/sandbox/src/replay_layer.rs` — 修改返回类型
- `crates/platform-api/src/routes/sandbox.rs` — 修改 replay 分支响应

**对现有功能的影响：** **低影响**。404 响应体结构变化，但 HTTP 状态码不变，下游按状态码判断的逻辑不受影响。

**风险：** 低。

---

### 15. 推送历史和成功率 — P3

**当前状态：** 推送成功/失败仅有日志，无持久化记录。

**实现方案：**

新增 `push_logs` 表：
```sql
CREATE TABLE push_logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    subscription_id UUID REFERENCES webhook_subscriptions(id),
    event_type VARCHAR(100),
    status VARCHAR(20), -- success / failed
    response_code INT,
    error_message TEXT,
    duration_ms INT,
    created_at TIMESTAMPTZ DEFAULT now()
);
```

PushDispatcher 每次推送后写入 push_logs。
API：`GET /api/v1/webhooks/{id}/logs` + `GET /api/v1/webhooks/{id}/stats`

**影响范围：**
- DB migration 新增表
- `crates/compensation/src/push_dispatcher.rs` — 添加日志记录
- `crates/platform-api/src/routes/webhooks.rs` — 新增端点

**对现有功能的影响：** **低影响**。推送逻辑新增写入步骤，失败不影响推送本身。

**风险：** 低。

---

### 16. OTel Tail Sampling — P3

**当前状态：** Collector 配置了 batch processor 但无 tail sampling。

**实现方案：**

修改 `docker/otel-collector-config.yml`：

```yaml
processors:
  batch:
    timeout: 5s
  tail_sampling:
    decision_wait: 10s
    policies:
      - name: error-policy
        type: status_code
        status_code: { status_codes: [ERROR] }
      - name: slow-policy
        type: latency
        latency: { threshold_ms: 3000 }
      - name: probabilistic-policy
        type: probabilistic
        probabilistic: { sampling_percentage: 10 }

service:
  pipelines:
    traces:
      processors: [tail_sampling, batch]
```

**影响范围：** 仅修改 Docker 配置文件。

**对现有功能的影响：** **零影响**（容器配置变更）。

**风险：** 低。但 tail sampling 会增加 Collector 内存使用（需要缓存 10s 的 span 数据）。

---

## 影响评估总结

### 零影响项（9 个）— 纯新增，不修改现有代码
| # | 项目 |
|---|------|
| 3 | 路由热加载轮询（新增后台任务） |
| 5 | 数据染色（proxy 层小修改） |
| 7 | Artifact 管理（新增 API） |
| 8 | 修改 payload 重推（新增 API） |
| 10 | K8s 部署清单（新增文件） |
| 11 | Rust Dockerfile（新增文件） |
| 13 | OData 支持（新增协议） |
| 14 | Replay 提示（扩展响应体） |
| 16 | Tail Sampling（Docker 配置） |

### 低影响项（4 个）— 新增字段或方法，不改变现有行为
| # | 项目 | 兼容策略 |
|---|------|---------|
| 4 | 加密存储 | 无 ENCRYPTION_KEY 时保持明文 |
| 12 | 推送格式 | 默认 JSON，现有订阅不变 |
| 15 | 推送历史 | 新增表和写入，不影响推送逻辑 |
| 1 | TLS | 无证书时保持 HTTP |

### 需谨慎处理项（3 个）— 影响请求链路或跨模块
| # | 项目 | 风险点 | 缓解措施 |
|---|------|--------|---------|
| 2 | JWT Auth | 中间件位于全局链路 | `AUTH_ENABLED=false` 默认关闭 |
| 6 | WebSocket | 前端架构变更 | 增量添加，不改现有页面逻辑 |
| 9 | OTel 指标 | dispatcher 签名变更 | metrics 参数可选，None 时不记录 |

---

## 建议实施顺序

```
Sprint 1 (P1 — 生产必需):
  #3 路由热加载轮询 (0.5天) → #1 TLS (1天) → #2 JWT Auth (2天)

Sprint 2 (P2 — 运维基础):
  #11 Dockerfile (0.5天) → #10 K8s (1天) → #9 OTel 指标 (1.5天) → #4 加密 (1天)

Sprint 3 (P3 — 体验增强):
  #8 payload 修改 (0.5天) → #5 数据染色 (0.5天) → #15 推送历史 (1天) →
  #12 推送格式 (1天) → #7 Artifact (1天) → #16 Tail Sampling (0.5天) →
  #6 WebSocket (2天)

Sprint 4 (P4 — 锦上添花):
  #14 Replay 提示 (0.5天) → #13 OData (3天)
```

总工作量约 **17.5 天**，按 Sprint 分批交付不阻塞生产。
