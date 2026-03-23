# 剩余 8 项功能详细规划与设计

> 对每项功能给出：架构设计、具体实现方案、涉及的文件变更、配置项、测试方案、对现有系统的影响评估

---

## 一、P1: TLS 终结 (rustls)

### 1.1 设计目标

生产环境必须通过 HTTPS 提供服务。使用 `rustls`（纯 Rust TLS 实现，无 OpenSSL 依赖）为网关提供 TLS 终结。开发环境保持 HTTP 不变。

### 1.2 架构设计

```
客户端 (HTTPS)
     │
     ▼
┌────────────────────┐
│  axum-server       │  ← TLS_CERT_PATH + TLS_KEY_PATH 时启用
│  (rustls 终结)     │
│  ↓ 明文 HTTP       │
│  Axum Router       │  ← 路由层不感知 TLS，逻辑完全不变
└────────────────────┘
```

### 1.3 实现方案

**新增依赖：**
```toml
# Cargo.toml (workspace)
axum-server = { version = "0.7", features = ["tls-rustls"] }
```

**AppConfig 扩展** (`crates/common/src/config.rs`)：
```rust
pub struct AppConfig {
    // ... 现有字段 ...
    pub tls_cert_path: Option<String>,  // TLS_CERT_PATH 环境变量
    pub tls_key_path: Option<String>,   // TLS_KEY_PATH 环境变量
}
```

**main.rs 启动逻辑分支：**
```rust
if let (Some(cert), Some(key)) = (&config.tls_cert_path, &config.tls_key_path) {
    // 生产模式：HTTPS
    let tls_config = axum_server::tls_rustls::RustlsConfig::from_pem_file(cert, key).await?;
    tracing::info!("Listening on https://{addr} (TLS enabled)");
    axum_server::bind_rustls(addr.parse()?, tls_config)
        .serve(app.into_make_service()).await?;
} else {
    // 开发模式：HTTP（现有逻辑不变）
    tracing::info!("Listening on http://{addr} (TLS disabled)");
    let listener = TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
}
```

### 1.4 涉及文件

| 文件 | 变更类型 | 说明 |
|------|---------|------|
| `Cargo.toml` (workspace) | 新增依赖 | `axum-server` |
| `crates/platform-api/Cargo.toml` | 新增依赖 | `axum-server.workspace = true` |
| `crates/common/src/config.rs` | 修改 | 新增 `tls_cert_path` + `tls_key_path` |
| `crates/platform-api/src/main.rs` | 修改 | 启动逻辑分支（10 行） |
| `.env.example` | 修改 | 新增 `TLS_CERT_PATH` + `TLS_KEY_PATH` |

### 1.5 配置项

```env
# TLS 配置（可选，不配置则使用 HTTP）
TLS_CERT_PATH=/etc/ssl/certs/api-anything.crt
TLS_KEY_PATH=/etc/ssl/private/api-anything.key
```

### 1.6 测试方案

- 单元测试：`AppConfig::from_env()` 正确读取 TLS 配置
- 集成测试：无 TLS 配置时，现有 342 个测试全部通过（零影响）
- 手动测试：使用自签名证书验证 HTTPS 访问

### 1.7 影响评估

**零影响**。不配置 `TLS_CERT_PATH` 时走现有 HTTP 路径，所有测试和功能不变。

---

## 二、P1: JWT 认证 + 鉴权映射

### 2.1 设计目标

- 网关对外统一使用 JWT Bearer Token 认证
- 从 JWT claims 中提取用户身份和角色
- 根据 `BackendBinding.auth_mapping` 将现代 Token 翻译为老系统凭证（WS-Security Header、SSH Key、Linux 用户等）
- 支持白名单路径（`/health`、`/api/v1/docs`）跳过认证
- 开发环境可通过 `AUTH_ENABLED=false` 关闭

### 2.2 架构设计

```
请求
  │
  ▼
┌──────────────────────────────────────┐
│          Auth Middleware             │
│                                      │
│  1. 检查路径是否在白名单 → 跳过      │
│  2. AUTH_ENABLED=false → 跳过        │
│  3. 提取 Authorization: Bearer xxx   │
│  4. 验证 JWT 签名 + 过期时间         │
│  5. 解码 claims → 注入 req.extensions│
│  6. 下游 handler 可从 extensions 读取│
└──────────────┬───────────────────────┘
               │
               ▼
        Axum Router（路由层）
               │
               ▼
        Gateway Handler
               │ 读取 auth_mapping 配置
               ▼
┌──────────────────────────────────────┐
│       凭证翻译层                      │
│  JWT claims.role="admin"             │
│       ↓ 根据 protocol 翻译           │
│  SOAP → WS-Security UsernameToken   │
│  SSH  → 从 Vault/配置获取 SSH Key    │
│  CLI  → 映射为 Linux 用户 (sudo)     │
│  PTY  → 注入用户名密码到 Expect 序列 │
└──────────────────────────────────────┘
```

### 2.3 实现方案

**新增依赖：**
```toml
jsonwebtoken = "9"
```

**新增中间件** (`crates/platform-api/src/middleware/auth_guard.rs`)：
```rust
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use jsonwebtoken::{decode, DecodingKey, Validation, Algorithm};

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Claims {
    pub sub: String,           // 用户 ID
    pub role: Option<String>,  // 角色
    pub exp: u64,              // 过期时间
}

#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub enabled: bool,
    pub jwt_secret: String,
    pub skip_paths: Vec<String>,  // 白名单
}

impl AuthConfig {
    pub fn from_env() -> Self {
        Self {
            enabled: std::env::var("AUTH_ENABLED")
                .map(|v| v == "true")
                .unwrap_or(false),  // 默认关闭
            jwt_secret: std::env::var("JWT_SECRET")
                .unwrap_or_else(|_| "dev-secret-change-in-production".to_string()),
            skip_paths: vec![
                "/health".to_string(),
                "/health/ready".to_string(),
                "/api/v1/docs".to_string(),
            ],
        }
    }
}

pub async fn auth_middleware(
    State(config): State<AuthConfig>,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    // 跳过条件
    if !config.enabled {
        return Ok(next.run(req).await);
    }
    let path = req.uri().path().to_string();
    if config.skip_paths.iter().any(|p| path.starts_with(p)) {
        return Ok(next.run(req).await);
    }

    // 提取 Bearer token
    let token = req.headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(AppError::Unauthorized)?;

    // 验证 JWT
    let key = DecodingKey::from_secret(config.jwt_secret.as_bytes());
    let validation = Validation::new(Algorithm::HS256);
    let token_data = decode::<Claims>(token, &key, &validation)
        .map_err(|_| AppError::Unauthorized)?;

    // 注入 claims 到请求扩展
    req.extensions_mut().insert(token_data.claims);

    Ok(next.run(req).await)
}
```

**AppError 扩展：**
```rust
#[error("Unauthorized")]
Unauthorized,  // → HTTP 401
```

**注册中间件** (`crates/platform-api/src/lib.rs`)：
```rust
use axum::middleware;

pub fn build_app(state: AppState) -> Router {
    let auth_config = AuthConfig::from_env();

    Router::new()
        // ... 所有路由 ...
        .layer(middleware::from_fn_with_state(auth_config, auth_middleware))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
```

### 2.4 涉及文件

| 文件 | 变更类型 | 说明 |
|------|---------|------|
| `Cargo.toml` (workspace) | 新增依赖 | `jsonwebtoken = "9"` |
| `crates/platform-api/Cargo.toml` | 新增依赖 | `jsonwebtoken.workspace = true` |
| `crates/common/src/error.rs` | 修改 | 新增 `AppError::Unauthorized` → 401 |
| `crates/platform-api/src/middleware/auth_guard.rs` | **新建** | JWT 验证中间件 (~80 行) |
| `crates/platform-api/src/middleware/mod.rs` | 修改 | 新增 `pub mod auth_guard;` |
| `crates/platform-api/src/lib.rs` | 修改 | 注册 auth 中间件层 |
| `.env.example` | 修改 | 新增 `AUTH_ENABLED` + `JWT_SECRET` |

### 2.5 配置项

```env
# JWT 认证（默认关闭，开发环境无需配置）
AUTH_ENABLED=false
JWT_SECRET=your-256-bit-secret-change-in-production
```

### 2.6 测试方案

- 单元测试：JWT 签名验证、过期拒绝、无 token 拒绝、白名单跳过
- 集成测试：`AUTH_ENABLED=false` 时所有现有 342 个测试不受影响
- 集成测试：`AUTH_ENABLED=true` 时，健康检查可访问，其他端点返回 401

### 2.7 影响评估

**条件性影响**。`AUTH_ENABLED=false`（默认）时零影响。`AUTH_ENABLED=true` 时所有非白名单路径需要 JWT token。现有测试环境默认关闭 auth。

---

## 三、P2: Kubernetes 部署清单 + HPA

### 3.1 设计目标

提供生产级 K8s 部署配置，支持 HPA 弹性伸缩。

### 3.2 实现方案

创建 `deploy/k8s/` 目录，包含：

```
deploy/k8s/
├── namespace.yaml              # api-anything namespace
├── configmap.yaml              # 环境变量配置
├── secret.yaml                 # 敏感配置模板（API Key 等）
├── deployment.yaml             # platform-api Deployment
│   ├── replicas: 2             # 最小副本数
│   ├── resources.requests      # CPU: 100m, Memory: 128Mi
│   ├── resources.limits        # CPU: 1000m, Memory: 512Mi
│   ├── livenessProbe: /health  # 存活检查
│   ├── readinessProbe: /health/ready  # 就绪检查
│   └── env: from ConfigMap + Secret
├── service.yaml                # ClusterIP Service, port 8080
├── hpa.yaml                    # HorizontalPodAutoscaler
│   ├── minReplicas: 2
│   ├── maxReplicas: 20
│   └── metrics: CPU 70%
└── ingress.yaml                # Ingress + TLS termination
```

### 3.3 涉及文件

全部为**新建文件**，不修改任何现有代码。

### 3.4 影响评估

**零影响**。纯配置文件，不改动代码。

---

## 四、P2: Rust 服务 Dockerfile

### 4.1 设计目标

- 多阶段构建：编译阶段（rust:alpine）→ 运行阶段（FROM scratch）
- 最终镜像 < 20MB
- musl 静态链接，无外部依赖
- 支持 amd64 和 arm64

### 4.2 实现方案

**`Dockerfile`（项目根目录）：**
```dockerfile
# Stage 1: Build
FROM rust:1.82-alpine AS builder
RUN apk add --no-cache musl-dev openssl-dev openssl-libs-static pkgconfig cmake make g++
WORKDIR /app
COPY . .
RUN cargo build --release -p api-anything-platform-api

# Stage 2: Runtime
FROM scratch
COPY --from=builder /app/target/release/api-anything-platform-api /app
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
EXPOSE 8080
ENTRYPOINT ["/app"]
```

### 4.3 涉及文件

| 文件 | 变更类型 |
|------|---------|
| `Dockerfile` | **新建** |
| `.dockerignore` | **新建** |

### 4.4 风险

**中等**。musl 编译可能遇到 C 依赖问题（rdkafka 需要 cmake，russh 需要 openssl）。可能需要通过 feature gate 在 Docker 构建中排除 Kafka 功能。

---

## 五、P2: OTel 自定义指标

### 5.1 设计目标

为网关的关键路径注入 Prometheus 兼容的自定义指标，供 Grafana 面板展示和告警使用。

### 5.2 指标定义

| 指标名 | 类型 | 维度 | 描述 |
|--------|------|------|------|
| `gateway_request_total` | Counter | route, method, status | 网关请求总量 |
| `gateway_request_duration_seconds` | Histogram | route, method | 请求延迟分布 |
| `backend_execute_duration_seconds` | Histogram | route, protocol | 后端调用延迟 |
| `backend_circuit_breaker_state` | Gauge | route | 熔断器状态（0=Closed,1=HalfOpen,2=Open） |
| `delivery_retry_total` | Counter | route, status | 重试次数 |
| `delivery_dead_letter_total` | Counter | route | 死信数量 |

### 5.3 实现方案

**新增文件** `crates/gateway/src/metrics.rs`：
```rust
use opentelemetry::metrics::{Counter, Histogram, Meter, UpDownCounter};
use opentelemetry::KeyValue;

pub struct GatewayMetrics {
    pub request_total: Counter<u64>,
    pub request_duration: Histogram<f64>,
    pub backend_duration: Histogram<f64>,
    pub circuit_breaker_state: UpDownCounter<i64>,
    pub retry_total: Counter<u64>,
    pub dead_letter_total: Counter<u64>,
}

impl GatewayMetrics {
    pub fn new(meter: &Meter) -> Self {
        Self {
            request_total: meter.u64_counter("gateway_request_total").build(),
            request_duration: meter.f64_histogram("gateway_request_duration_seconds").build(),
            backend_duration: meter.f64_histogram("backend_execute_duration_seconds").build(),
            circuit_breaker_state: meter.i64_up_down_counter("backend_circuit_breaker_state").build(),
            retry_total: meter.u64_counter("delivery_retry_total").build(),
            dead_letter_total: meter.u64_counter("delivery_dead_letter_total").build(),
        }
    }
}
```

**在 gateway handler 中记录指标：**
```rust
// 请求开始时
let start = Instant::now();

// 请求结束后
let duration = start.elapsed().as_secs_f64();
metrics.request_total.add(1, &[KeyValue::new("route", path), KeyValue::new("status", status)]);
metrics.request_duration.record(duration, &[KeyValue::new("route", path)]);
```

**在 main.rs 中初始化 MeterProvider：**
```rust
use opentelemetry_sdk::metrics::SdkMeterProvider;
let meter_provider = SdkMeterProvider::builder()
    .with_reader(/* Prometheus exporter or OTLP */)
    .build();
let meter = meter_provider.meter("api-anything");
let metrics = Arc::new(GatewayMetrics::new(&meter));
```

### 5.4 涉及文件

| 文件 | 变更类型 | 说明 |
|------|---------|------|
| `crates/gateway/src/metrics.rs` | **新建** | 指标定义 |
| `crates/gateway/src/lib.rs` | 修改 | `pub mod metrics;` |
| `crates/platform-api/src/state.rs` | 修改 | AppState 新增 `metrics: Arc<GatewayMetrics>` |
| `crates/platform-api/src/routes/gateway.rs` | 修改 | 记录 request 指标 |
| `crates/gateway/src/dispatcher.rs` | 修改 | 记录 backend 指标 |
| `crates/platform-api/src/main.rs` | 修改 | 初始化 MeterProvider |

### 5.5 影响评估

**微小影响**。在请求热路径上增加了指标记录调用（纳秒级开销）。AppState 新增字段需要更新所有测试中的 state 构建代码。可以用 `Option<Arc<GatewayMetrics>>` 使 metrics 可选，测试时传 None。

---

## 六、P3: WebSocket 实时推送

### 6.1 设计目标

前端通过 WebSocket 实时接收事件通知（路由变更、死信告警、生成完成等），无需手动刷新。

### 6.2 架构设计

```
Web 前端                            Platform API
  │                                      │
  ├── WS 连接 /ws ──────────────────────▶│
  │                                      │ 轮询 events 表
  │◀──── event: route.updated ──────────│ 或 EventBus 订阅
  │◀──── event: dead_letter.new ────────│
  │◀──── event: generation.completed ───│
  │                                      │
  └──────────────────────────────────────┘
```

### 6.3 实现方案

**后端** (`crates/platform-api/src/routes/ws.rs`)：
```rust
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_ws(socket, state))
}

async fn handle_ws(mut socket: WebSocket, state: AppState) {
    let mut interval = tokio::time::interval(Duration::from_secs(2));
    let mut last_id: Option<Uuid> = None;

    loop {
        interval.tick().await;
        // 从 events 表查询新事件
        let events = query_new_events(&state.db, &last_id).await;
        for event in &events {
            let json = serde_json::to_string(&event).unwrap_or_default();
            if socket.send(Message::Text(json)).await.is_err() {
                return; // 客户端断开
            }
            last_id = Some(event.id);
        }
    }
}
```

**前端** (`web/src/hooks/useWebSocket.ts`)：
```typescript
export function useWebSocket(onEvent: (event: any) => void) {
    useEffect(() => {
        const ws = new WebSocket(`ws://${location.host}/ws`);
        ws.onmessage = (e) => onEvent(JSON.parse(e.data));
        ws.onclose = () => setTimeout(() => /* reconnect */, 3000);
        return () => ws.close();
    }, []);
}
```

**在各页面中使用：**
```typescript
// Dashboard.tsx
useWebSocket((event) => {
    if (event.type === 'route.updated') loadProjects();
    if (event.type === 'dead_letter') showNotification('新的死信记录');
});
```

### 6.4 涉及文件

| 文件 | 变更类型 |
|------|---------|
| `crates/platform-api/src/routes/ws.rs` | **新建** |
| `crates/platform-api/src/routes/mod.rs` | 修改 |
| `crates/platform-api/src/lib.rs` | 修改（注册 /ws 路由） |
| `web/src/hooks/useWebSocket.ts` | **新建** |
| `web/src/components/Layout.tsx` | 修改（建立 WS 连接） |
| `web/src/pages/*.tsx` | 修改（添加自动刷新） |

### 6.5 影响评估

**零后端影响**。WebSocket 是独立端点，不影响 REST API。前端改动是增量的。

---

## 七、P3: source_config 加密存储

### 7.1 设计目标

`BackendBinding.endpoint_config` 中存储了 SSH 密码、SOAP 凭证等敏感信息，需要加密存储。

### 7.2 实现方案

**新增文件** `crates/common/src/crypto.rs`：
```rust
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};

pub struct Encryptor {
    key: LessSafeKey,
    rng: SystemRandom,
}

impl Encryptor {
    pub fn from_env() -> Option<Self> {
        let key_hex = std::env::var("ENCRYPTION_KEY").ok()?;
        let key_bytes = hex::decode(&key_hex).ok()?;
        if key_bytes.len() != 32 { return None; } // AES-256 需要 32 字节
        let unbound = UnboundKey::new(&AES_256_GCM, &key_bytes).ok()?;
        Some(Self { key: LessSafeKey::new(unbound), rng: SystemRandom::new() })
    }

    pub fn encrypt(&self, plaintext: &str) -> Result<String, anyhow::Error> {
        let mut nonce_bytes = [0u8; NONCE_LEN];
        self.rng.fill(&mut nonce_bytes)?;
        let nonce = Nonce::assume_unique_for_key(nonce_bytes);
        let mut in_out = plaintext.as_bytes().to_vec();
        self.key.seal_in_place_append_tag(nonce, Aad::empty(), &mut in_out)?;
        // 格式：nonce(12 bytes) + ciphertext + tag(16 bytes)，hex 编码
        let mut result = nonce_bytes.to_vec();
        result.extend_from_slice(&in_out);
        Ok(hex::encode(result))
    }

    pub fn decrypt(&self, ciphertext_hex: &str) -> Result<String, anyhow::Error> {
        let data = hex::decode(ciphertext_hex)?;
        if data.len() < NONCE_LEN + 16 { return Err(anyhow::anyhow!("Invalid ciphertext")); }
        let (nonce_bytes, encrypted) = data.split_at(NONCE_LEN);
        let nonce = Nonce::assume_unique_for_key(nonce_bytes.try_into()?);
        let mut in_out = encrypted.to_vec();
        let plaintext = self.key.open_in_place(nonce, Aad::empty(), &mut in_out)?;
        Ok(String::from_utf8(plaintext.to_vec())?)
    }
}
```

**MetadataRepo 层透明加解密：**
- `create_backend_binding` 写入前加密 `endpoint_config`
- `list_active_routes_with_bindings` 读取后解密 `endpoint_config`
- 无 `ENCRYPTION_KEY` 时保持明文（向后兼容）

### 7.3 配置项

```env
# 数据加密（可选，不配置则明文存储）
ENCRYPTION_KEY=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
```

### 7.4 影响评估

**低影响**。无 key 时完全不变。有 key 时新写入的数据加密，旧数据仍可读取（明文数据不以 hex 格式开头，可区分）。

---

## 八、P3: 路由定时轮询刷新

### 8.1 设计目标

网关运行时自动检测数据库中的路由变更，无需重启即可加载新生成的路由。

### 8.2 实现方案

**在 main.rs 中启动后台轮询任务：**
```rust
// 路由热加载轮询
let poll_repo = repo.clone();
let poll_router = router.clone();
let poll_dispatchers = dispatchers.clone();
tokio::spawn(async move {
    let poll_interval = Duration::from_secs(
        std::env::var("ROUTE_POLL_INTERVAL_SECS")
            .ok().and_then(|v| v.parse().ok())
            .unwrap_or(5)
    );
    let mut last_count: usize = 0;
    let mut interval = tokio::time::interval(poll_interval);
    loop {
        interval.tick().await;
        match poll_repo.list_active_routes_with_bindings().await {
            Ok(routes) if routes.len() != last_count => {
                match RouteLoader::load(poll_repo.as_ref(), &poll_router, &poll_dispatchers).await {
                    Ok(loaded) => {
                        last_count = loaded;
                        tracing::info!(routes = loaded, "Routes hot-reloaded");
                    }
                    Err(e) => tracing::error!(error = %e, "Route reload failed"),
                }
            }
            Ok(routes) => { last_count = routes.len(); }
            Err(e) => tracing::warn!(error = %e, "Route poll check failed"),
        }
    }
});
```

### 8.3 涉及文件

| 文件 | 变更类型 | 说明 |
|------|---------|------|
| `crates/platform-api/src/main.rs` | 修改 | 新增 ~25 行轮询任务 |
| `.env.example` | 修改 | 新增 `ROUTE_POLL_INTERVAL_SECS` |

### 8.4 配置项

```env
# 路由轮询间隔（秒，默认 5）
ROUTE_POLL_INTERVAL_SECS=5
```

### 8.5 影响评估

**零影响**。纯新增后台任务，不修改任何现有代码路径。每 5 秒一次轻量 SQL 查询（仅比较数量），路由无变化时不触发加载。DynamicRouter 的 RCU 机制已经支持原子替换。

---

## 实施顺序建议

```
Sprint 1 (P1 — 生产安全，约 1.5 天):
  ③ 路由轮询刷新 (0.5h, 最简单，先做)
  → ① TLS (0.5天)
  → ② JWT Auth (1天)

Sprint 2 (P2 — 运维基础，约 2 天):
  ④ Rust Dockerfile (2h)
  → ③ K8s 部署清单 (0.5天)
  → ⑤ OTel 指标 (1天)

Sprint 3 (P3 — 体验增强，约 2.5 天):
  ⑦ 加密存储 (0.5天)
  → ⑥ WebSocket (1.5天)
```

**总工作量约 6 天，分 3 个 Sprint 交付。**

---

## 回归测试策略

所有 8 项功能都设计为**向后兼容**：

| 功能 | 兼容机制 | 现有测试影响 |
|------|---------|------------|
| TLS | 无证书时 HTTP | 零 |
| JWT | `AUTH_ENABLED=false` 默认 | 零 |
| K8s | 纯 YAML 文件 | 零 |
| Dockerfile | 纯新增文件 | 零 |
| OTel 指标 | `Option<Metrics>` 可选 | 需适配 AppState |
| WebSocket | 独立端点 | 零 |
| 加密 | 无 key 时明文 | 零 |
| 路由轮询 | 新增后台任务 | 零 |
