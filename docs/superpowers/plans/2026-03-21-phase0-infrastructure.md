# Phase 0: 基础设施 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 搭建 API-Anything 平台的完整开发基础设施，包括 Rust workspace、元数据仓库、Platform API 骨架、本地 Docker 开发环境（PG + Kafka + OTel 全栈）和 CI/CD 流水线。

**Architecture:** Rust workspace 多 crate 结构，`common` 定义共享领域模型，`metadata` 封装 PostgreSQL 交互，`platform-api` 提供 Axum HTTP 服务，`cli` 提供命令行入口。所有服务通过 Docker Compose 在本地一键启动。

**Tech Stack:** Rust 1.82+, Axum 0.8, sqlx 0.8 (PostgreSQL), rdkafka, tracing + opentelemetry, Docker Compose, GitHub Actions

**Spec:** `docs/superpowers/specs/2026-03-21-api-anything-platform-design.md`

---

## File Structure

```
api-anything/
├── Cargo.toml                              # Workspace 根配置
├── rust-toolchain.toml                     # 锁定 Rust 版本
├── .gitignore
├── .env.example                            # 环境变量模板
│
├── crates/
│   ├── common/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs                      # 重导出所有子模块
│   │       ├── models.rs                   # 核心领域模型 (Project, Contract, Route 等)
│   │       ├── error.rs                    # 统一错误类型 (RFC 7807)
│   │       └── config.rs                   # 配置加载 (环境变量 + 文件)
│   │
│   ├── metadata/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs                      # 重导出 Repository trait + impl
│   │       ├── repo.rs                     # MetadataRepo trait 定义
│   │       ├── pg.rs                       # PostgreSQL 实现
│   │       └── migrations/                 # sqlx 版本化迁移
│   │           └── 20260321000000_initial_schema.sql
│   │
│   ├── platform-api/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs                     # 入口：启动 Axum server
│   │       ├── state.rs                    # AppState (DB pool, Kafka producer 等)
│   │       ├── routes/
│   │       │   ├── mod.rs                  # 路由注册
│   │       │   ├── health.rs               # /health 健康检查
│   │       │   └── projects.rs             # /api/v1/projects CRUD
│   │       └── middleware/
│   │           ├── mod.rs                  # 中间件注册
│   │           ├── tracing_mw.rs           # OTel tracing 中间件
│   │           └── error_handler.rs        # 全局错误处理 → RFC 7807
│   │
│   └── cli/
│       ├── Cargo.toml
│       └── src/
│           └── main.rs                     # CLI 入口骨架
│
├── docker/
│   ├── docker-compose.yml                  # PG + Kafka + Zookeeper + OTel Collector + Tempo + Prometheus + Loki + Grafana
│   ├── otel-collector-config.yml           # OTel Collector 配置
│   ├── prometheus.yml                      # Prometheus 抓取配置
│   ├── loki-config.yml                     # Loki 配置
│   ├── tempo-config.yml                    # Tempo 配置
│   └── grafana/
│       └── provisioning/
│           └── datasources/
│               └── datasources.yml         # Grafana 数据源自动配置
│
└── .github/
    └── workflows/
        └── ci.yml                          # GitHub Actions CI
```

---

### Task 1: Rust Workspace 脚手架

**Files:**
- Create: `Cargo.toml`
- Create: `rust-toolchain.toml`
- Create: `.gitignore`
- Create: `.env.example`
- Create: `crates/common/Cargo.toml`
- Create: `crates/common/src/lib.rs`
- Create: `crates/metadata/Cargo.toml`
- Create: `crates/metadata/src/lib.rs`
- Create: `crates/platform-api/Cargo.toml`
- Create: `crates/platform-api/src/main.rs`
- Create: `crates/cli/Cargo.toml`
- Create: `crates/cli/src/main.rs`

- [ ] **Step 1: 创建 workspace 根 Cargo.toml**

```toml
[workspace]
resolver = "2"
members = [
    "crates/common",
    "crates/metadata",
    "crates/platform-api",
    "crates/cli",
]

[workspace.package]
version = "0.1.0"
edition = "2021"
rust-version = "1.82"
license = "MIT"

[workspace.dependencies]
# Async runtime
tokio = { version = "1", features = ["full"] }

# Web framework
axum = { version = "0.8", features = ["macros"] }
tower = "0.5"
tower-http = { version = "0.6", features = ["trace", "cors", "compression-gzip"] }

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Database
sqlx = { version = "0.8", features = ["runtime-tokio", "tls-rustls", "postgres", "uuid", "chrono", "json"] }

# Kafka
rdkafka = { version = "0.37", features = ["cmake-build"] }

# Observability
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
opentelemetry = "0.27"
opentelemetry_sdk = { version = "0.27", features = ["rt-tokio"] }
opentelemetry-otlp = { version = "0.27", features = ["tonic"] }
tracing-opentelemetry = "0.28"

# Common utilities
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
thiserror = "2"
anyhow = "1"
dotenvy = "0.15"
clap = { version = "4", features = ["derive"] }

# Testing
assert_json_diff = "2"
```

- [ ] **Step 2: 创建 rust-toolchain.toml**

```toml
[toolchain]
channel = "1.82"
components = ["rustfmt", "clippy"]
```

- [ ] **Step 3: 更新 .gitignore**

追加 Rust 和项目相关的忽略规则到已有的 `.gitignore`：

```gitignore
# Rust
/target
**/*.rs.bk

# Environment
.env

# IDE
.idea/
.vscode/
*.swp

# OS
.DS_Store

# Docker volumes
docker/data/

# Generated artifacts
artifacts/
```

- [ ] **Step 4: 创建 .env.example**

```env
# PostgreSQL
DATABASE_URL=postgres://api_anything:api_anything@localhost:5432/api_anything

# Kafka
KAFKA_BROKERS=localhost:9092

# OTel
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317

# Server
API_HOST=0.0.0.0
API_PORT=8080

# Logging
RUST_LOG=api_anything=debug,tower_http=debug
```

- [ ] **Step 5: 创建 crates/common/Cargo.toml**

```toml
[package]
name = "api-anything-common"
version.workspace = true
edition.workspace = true

[dependencies]
serde.workspace = true
serde_json.workspace = true
uuid.workspace = true
chrono.workspace = true
thiserror.workspace = true
```

- [ ] **Step 6: 创建 crates/common/src/lib.rs（空骨架）**

```rust
pub mod config;
pub mod error;
pub mod models;
```

同时创建空的 `config.rs`、`error.rs`、`models.rs` 文件（内容在后续 Task 填充）。

- [ ] **Step 7: 创建 crates/metadata/Cargo.toml**

```toml
[package]
name = "api-anything-metadata"
version.workspace = true
edition.workspace = true

[dependencies]
api-anything-common = { path = "../common" }
sqlx.workspace = true
tokio.workspace = true
uuid.workspace = true
chrono.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
tracing.workspace = true

[dev-dependencies]
assert_json_diff.workspace = true
```

- [ ] **Step 8: 创建 crates/metadata/src/lib.rs（空骨架）**

```rust
pub mod pg;
pub mod repo;

pub use repo::MetadataRepo;
```

同时创建空的 `repo.rs`、`pg.rs` 文件。

- [ ] **Step 9: 创建 crates/platform-api/Cargo.toml**

```toml
[package]
name = "api-anything-platform-api"
version.workspace = true
edition.workspace = true

[dependencies]
api-anything-common = { path = "../common" }
api-anything-metadata = { path = "../metadata" }
axum.workspace = true
tokio.workspace = true
tower.workspace = true
tower-http.workspace = true
serde.workspace = true
serde_json.workspace = true
uuid.workspace = true
sqlx.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
opentelemetry.workspace = true
opentelemetry_sdk.workspace = true
opentelemetry-otlp.workspace = true
tracing-opentelemetry.workspace = true
dotenvy.workspace = true
anyhow.workspace = true

[dev-dependencies]
axum-test = "16"
assert_json_diff.workspace = true
```

- [ ] **Step 10: 创建 crates/platform-api/src/main.rs（最小可运行）**

```rust
#[tokio::main]
async fn main() {
    println!("API-Anything Platform API starting...");
}
```

- [ ] **Step 11: 创建 crates/cli/Cargo.toml**

```toml
[package]
name = "api-anything-cli"
version.workspace = true
edition.workspace = true

[dependencies]
api-anything-common = { path = "../common" }
clap.workspace = true
tokio.workspace = true
serde.workspace = true
serde_json.workspace = true
anyhow.workspace = true
```

- [ ] **Step 12: 创建 crates/cli/src/main.rs（最小骨架）**

```rust
use clap::Parser;

#[derive(Parser)]
#[command(name = "api-anything", about = "AI-powered legacy system API gateway generator")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Generate REST API from legacy system
    Generate {
        /// Path to source contract (WSDL, help output, etc.)
        #[arg(short, long)]
        source: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _cli = Cli::parse();
    println!("API-Anything CLI — not yet implemented");
    Ok(())
}
```

- [ ] **Step 13: 验证 workspace 编译**

Run: `cargo check --workspace`
Expected: 编译成功，无错误

- [ ] **Step 14: Commit**

```bash
git add -A
git commit -m "chore: initialize Rust workspace with 4 crates scaffold"
```

---

### Task 2: 核心领域模型 (common crate)

**Files:**
- Create: `crates/common/src/models.rs`
- Create: `crates/common/src/error.rs`
- Create: `crates/common/src/config.rs`
- Test: `crates/common/src/models.rs` (内联 #[cfg(test)])

- [ ] **Step 1: 编写 models.rs 的测试**

在 `crates/common/src/models.rs` 底部写内联测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_serialization_roundtrip() {
        let project = Project {
            id: uuid::Uuid::new_v4(),
            name: "legacy-soap-service".to_string(),
            description: "Legacy SOAP order service".to_string(),
            owner: "team-platform".to_string(),
            source_type: SourceType::Wsdl,
            source_config: serde_json::json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let json = serde_json::to_string(&project).unwrap();
        let deserialized: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(project.name, deserialized.name);
        assert_eq!(project.source_type, deserialized.source_type);
    }

    #[test]
    fn source_type_variants_serialize_as_lowercase() {
        assert_eq!(serde_json::to_string(&SourceType::Wsdl).unwrap(), "\"wsdl\"");
        assert_eq!(serde_json::to_string(&SourceType::Cli).unwrap(), "\"cli\"");
        assert_eq!(serde_json::to_string(&SourceType::Ssh).unwrap(), "\"ssh\"");
    }

    #[test]
    fn delivery_guarantee_default_is_at_most_once() {
        assert_eq!(DeliveryGuarantee::default(), DeliveryGuarantee::AtMostOnce);
    }

    #[test]
    fn http_method_includes_patch() {
        let method: HttpMethod = serde_json::from_str("\"PATCH\"").unwrap();
        assert_eq!(method, HttpMethod::Patch);
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p api-anything-common`
Expected: FAIL — 类型未定义

- [ ] **Step 3: 实现 models.rs**

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// --- 枚举类型 ---

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "source_type", rename_all = "snake_case")]
pub enum SourceType {
    Wsdl,
    Odata,
    Cli,
    Ssh,
    Pty,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "UPPERCASE")]
#[sqlx(type_name = "http_method", rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "contract_status", rename_all = "snake_case")]
pub enum ContractStatus {
    Draft,
    Active,
    Deprecated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "protocol_type", rename_all = "snake_case")]
pub enum ProtocolType {
    Soap,
    Http,
    Cli,
    Ssh,
    Pty,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "delivery_guarantee", rename_all = "snake_case")]
pub enum DeliveryGuarantee {
    #[default]
    AtMostOnce,
    AtLeastOnce,
    ExactlyOnce,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "artifact_type", rename_all = "snake_case")]
pub enum ArtifactType {
    PluginSo,
    ConfigYaml,
    OpenapiJson,
    Dockerfile,
    TestSuite,
    AgentPrompt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "build_status", rename_all = "snake_case")]
pub enum BuildStatus {
    Building,
    Ready,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "delivery_status", rename_all = "snake_case")]
pub enum DeliveryStatus {
    Pending,
    Delivered,
    Failed,
    Dead,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "sandbox_mode", rename_all = "snake_case")]
pub enum SandboxMode {
    Mock,
    Replay,
    Proxy,
}

// --- 领域模型 ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub owner: String,
    pub source_type: SourceType,
    pub source_config: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contract {
    pub id: Uuid,
    pub project_id: Uuid,
    pub version: String,
    pub status: ContractStatus,
    pub original_schema: String,
    pub parsed_model: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Route {
    pub id: Uuid,
    pub contract_id: Uuid,
    pub method: HttpMethod,
    pub path: String,
    pub request_schema: serde_json::Value,
    pub response_schema: serde_json::Value,
    pub transform_rules: serde_json::Value,
    pub backend_binding_id: Uuid,
    pub delivery_guarantee: DeliveryGuarantee,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendBinding {
    pub id: Uuid,
    pub protocol: ProtocolType,
    pub endpoint_config: serde_json::Value,
    pub connection_pool_config: serde_json::Value,
    pub circuit_breaker_config: serde_json::Value,
    pub rate_limit_config: serde_json::Value,
    pub retry_config: serde_json::Value,
    pub timeout_ms: i64,
    pub auth_mapping: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: Uuid,
    pub contract_id: Uuid,
    pub artifact_type: ArtifactType,
    pub content_hash: String,
    pub storage_path: String,
    pub build_status: BuildStatus,
    pub build_log: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryRecord {
    pub id: Uuid,
    pub route_id: Uuid,
    pub trace_id: String,
    pub idempotency_key: Option<String>,
    pub request_payload: serde_json::Value,
    pub response_payload: Option<serde_json::Value>,
    pub status: DeliveryStatus,
    pub retry_count: i32,
    pub next_retry_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxSession {
    pub id: Uuid,
    pub project_id: Uuid,
    pub tenant_id: String,
    pub mode: SandboxMode,
    pub config: serde_json::Value,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedInteraction {
    pub id: Uuid,
    pub session_id: Uuid,
    pub route_id: Uuid,
    pub request: serde_json::Value,
    pub response: serde_json::Value,
    pub duration_ms: i32,
    pub recorded_at: DateTime<Utc>,
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p api-anything-common`
Expected: 4 tests PASS

- [ ] **Step 5: 实现 error.rs**

```rust
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// RFC 7807 Problem Details 标准错误响应
#[derive(Debug, Serialize)]
pub struct ProblemDetail {
    #[serde(rename = "type")]
    pub error_type: String,
    pub title: String,
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
}

impl ProblemDetail {
    pub fn not_found(detail: impl Into<String>) -> Self {
        Self {
            error_type: "about:blank".to_string(),
            title: "Not Found".to_string(),
            status: 404,
            detail: Some(detail.into()),
            instance: None,
        }
    }

    pub fn bad_request(detail: impl Into<String>) -> Self {
        Self {
            error_type: "about:blank".to_string(),
            title: "Bad Request".to_string(),
            status: 400,
            detail: Some(detail.into()),
            instance: None,
        }
    }

    pub fn internal(detail: impl Into<String>) -> Self {
        Self {
            error_type: "about:blank".to_string(),
            title: "Internal Server Error".to_string(),
            status: 500,
            detail: Some(detail.into()),
            instance: None,
        }
    }
}

impl IntoResponse for ProblemDetail {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = serde_json::to_string(&self).unwrap_or_default();
        (
            status,
            [("content-type", "application/problem+json")],
            body,
        )
            .into_response()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::NotFound(msg) => ProblemDetail::not_found(msg).into_response(),
            AppError::BadRequest(msg) => ProblemDetail::bad_request(msg).into_response(),
            AppError::Database(e) => {
                tracing::error!(error = %e, "Database error");
                ProblemDetail::internal("Database error").into_response()
            }
            AppError::Internal(msg) => ProblemDetail::internal(msg).into_response(),
        }
    }
}
```

注意：需要在 `crates/common/Cargo.toml` 中添加 `axum` 和 `tracing` 依赖：

```toml
[dependencies]
# ... existing ...
axum.workspace = true
sqlx.workspace = true
tracing.workspace = true
```

- [ ] **Step 6: 实现 config.rs**

```rust
use std::env;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub database_url: String,
    pub kafka_brokers: String,
    pub otel_endpoint: String,
    pub api_host: String,
    pub api_port: u16,
}

impl AppConfig {
    /// 从环境变量加载配置，缺失时使用默认值
    pub fn from_env() -> Self {
        Self {
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://api_anything:api_anything@localhost:5432/api_anything".to_string()),
            kafka_brokers: env::var("KAFKA_BROKERS")
                .unwrap_or_else(|_| "localhost:9092".to_string()),
            otel_endpoint: env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
                .unwrap_or_else(|_| "http://localhost:4317".to_string()),
            api_host: env::var("API_HOST")
                .unwrap_or_else(|_| "0.0.0.0".to_string()),
            api_port: env::var("API_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8080),
        }
    }
}
```

- [ ] **Step 7: 验证编译**

Run: `cargo check --workspace`
Expected: 编译成功

- [ ] **Step 8: Commit**

```bash
git add crates/common/
git commit -m "feat(common): add core domain models, RFC 7807 error types, and config"
```

---

### Task 3: Docker Compose 本地开发环境

**Files:**
- Create: `docker/docker-compose.yml`
- Create: `docker/otel-collector-config.yml`
- Create: `docker/prometheus.yml`
- Create: `docker/tempo-config.yml`
- Create: `docker/loki-config.yml`
- Create: `docker/grafana/provisioning/datasources/datasources.yml`

- [ ] **Step 1: 创建 docker-compose.yml**

```yaml
services:
  # --- 数据存储 ---
  postgres:
    image: postgres:16-alpine
    environment:
      POSTGRES_USER: api_anything
      POSTGRES_PASSWORD: api_anything
      POSTGRES_DB: api_anything
    ports:
      - "5432:5432"
    volumes:
      - pg_data:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U api_anything"]
      interval: 5s
      timeout: 5s
      retries: 5

  # --- 消息队列 ---
  zookeeper:
    image: confluentinc/cp-zookeeper:7.7.0
    environment:
      ZOOKEEPER_CLIENT_PORT: 2181
      ZOOKEEPER_TICK_TIME: 2000

  kafka:
    image: confluentinc/cp-kafka:7.7.0
    depends_on:
      - zookeeper
    ports:
      - "9092:9092"
    environment:
      KAFKA_BROKER_ID: 1
      KAFKA_ZOOKEEPER_CONNECT: zookeeper:2181
      KAFKA_ADVERTISED_LISTENERS: PLAINTEXT://localhost:9092
      KAFKA_OFFSETS_TOPIC_REPLICATION_FACTOR: 1
      KAFKA_AUTO_CREATE_TOPICS_ENABLE: "true"
    healthcheck:
      test: ["CMD", "kafka-topics", "--bootstrap-server", "localhost:9092", "--list"]
      interval: 10s
      timeout: 10s
      retries: 5

  # --- 可观测性 ---
  otel-collector:
    image: otel/opentelemetry-collector-contrib:0.114.0
    command: ["--config=/etc/otel-collector-config.yml"]
    volumes:
      - ./otel-collector-config.yml:/etc/otel-collector-config.yml
    ports:
      - "4317:4317"   # OTLP gRPC
      - "4318:4318"   # OTLP HTTP
    depends_on:
      - tempo
      - prometheus

  tempo:
    image: grafana/tempo:2.6.1
    command: ["-config.file=/etc/tempo-config.yml"]
    volumes:
      - ./tempo-config.yml:/etc/tempo-config.yml
      - tempo_data:/var/tempo
    ports:
      - "3200:3200"   # Tempo query

  prometheus:
    image: prom/prometheus:v2.55.1
    command:
      - "--config.file=/etc/prometheus/prometheus.yml"
      - "--enable-feature=remote-write-receiver"
    volumes:
      - ./prometheus.yml:/etc/prometheus/prometheus.yml
      - prom_data:/prometheus
    ports:
      - "9090:9090"

  loki:
    image: grafana/loki:3.3.2
    command: ["-config.file=/etc/loki/loki-config.yml"]
    volumes:
      - ./loki-config.yml:/etc/loki/loki-config.yml
      - loki_data:/loki
    ports:
      - "3100:3100"

  grafana:
    image: grafana/grafana:11.4.0
    environment:
      GF_SECURITY_ADMIN_USER: admin
      GF_SECURITY_ADMIN_PASSWORD: admin
      GF_AUTH_ANONYMOUS_ENABLED: "true"
      GF_AUTH_ANONYMOUS_ORG_ROLE: Admin
    volumes:
      - ./grafana/provisioning:/etc/grafana/provisioning
      - grafana_data:/var/lib/grafana
    ports:
      - "3000:3000"
    depends_on:
      - prometheus
      - tempo
      - loki

volumes:
  pg_data:
  tempo_data:
  prom_data:
  loki_data:
  grafana_data:
```

- [ ] **Step 2: 创建 otel-collector-config.yml**

```yaml
receivers:
  otlp:
    protocols:
      grpc:
        endpoint: 0.0.0.0:4317
      http:
        endpoint: 0.0.0.0:4318

processors:
  batch:
    timeout: 5s
    send_batch_size: 1024

exporters:
  otlp/tempo:
    endpoint: tempo:4317
    tls:
      insecure: true

  prometheusremotewrite:
    endpoint: http://prometheus:9090/api/v1/write

  loki:
    endpoint: http://loki:3100/loki/api/v1/push

service:
  pipelines:
    traces:
      receivers: [otlp]
      processors: [batch]
      exporters: [otlp/tempo]
    metrics:
      receivers: [otlp]
      processors: [batch]
      exporters: [prometheusremotewrite]
    logs:
      receivers: [otlp]
      processors: [batch]
      exporters: [loki]
```

- [ ] **Step 3: 创建 prometheus.yml**

```yaml
global:
  scrape_interval: 15s
  evaluation_interval: 15s

scrape_configs:
  - job_name: "otel-collector"
    static_configs:
      - targets: ["otel-collector:8888"]

  - job_name: "api-anything"
    static_configs:
      - targets: ["host.docker.internal:8080"]
```

- [ ] **Step 4: 创建 tempo-config.yml**

```yaml
server:
  http_listen_port: 3200

distributor:
  receivers:
    otlp:
      protocols:
        grpc:
          endpoint: 0.0.0.0:4317

storage:
  trace:
    backend: local
    local:
      path: /var/tempo/traces
    wal:
      path: /var/tempo/wal

metrics_generator:
  storage:
    path: /var/tempo/metrics
```

- [ ] **Step 5: 创建 loki-config.yml**

```yaml
auth_enabled: false

server:
  http_listen_port: 3100

common:
  path_prefix: /loki
  storage:
    filesystem:
      chunks_directory: /loki/chunks
      rules_directory: /loki/rules
  replication_factor: 1
  ring:
    kvstore:
      store: inmemory

schema_config:
  configs:
    - from: "2024-01-01"
      store: tsdb
      object_store: filesystem
      schema: v13
      index:
        prefix: index_
        period: 24h
```

- [ ] **Step 6: 创建 Grafana datasources 自动配置**

```yaml
apiVersion: 1

datasources:
  - name: Prometheus
    type: prometheus
    access: proxy
    url: http://prometheus:9090
    isDefault: true

  - name: Tempo
    type: tempo
    access: proxy
    url: http://tempo:3200
    jsonData:
      tracesToLogsV2:
        datasourceUid: loki
        filterByTraceID: true
      tracesToMetrics:
        datasourceUid: prometheus

  - name: Loki
    type: loki
    access: proxy
    url: http://loki:3100
    uid: loki
    jsonData:
      derivedFields:
        - datasourceUid: tempo
          matcherRegex: "trace_id=(\\w+)"
          name: TraceID
          url: "$${__value.raw}"
```

- [ ] **Step 7: 启动并验证所有服务**

Run: `cd docker && docker compose up -d`
Expected: 所有服务启动成功

验证各服务:
- PostgreSQL: `docker compose exec postgres pg_isready -U api_anything` → 返回 accepting connections
- Kafka: `docker compose exec kafka kafka-topics --bootstrap-server localhost:9092 --list` → 无报错
- Grafana: 浏览器访问 `http://localhost:3000` → 可登录

- [ ] **Step 8: Commit**

```bash
git add docker/
git commit -m "infra: add Docker Compose with PG, Kafka, OTel Collector, Tempo, Prometheus, Loki, Grafana"
```

---

### Task 4: PostgreSQL 初始 Schema + Migration

**Files:**
- Create: `crates/metadata/src/migrations/20260321000000_initial_schema.sql`
- Modify: `crates/metadata/src/pg.rs`
- Modify: `crates/metadata/src/repo.rs`
- Modify: `crates/metadata/src/lib.rs`

- [ ] **Step 1: 编写 20260321000000_initial_schema.sql**

```sql
-- 自定义枚举类型
CREATE TYPE source_type AS ENUM ('wsdl', 'odata', 'cli', 'ssh', 'pty');
CREATE TYPE contract_status AS ENUM ('draft', 'active', 'deprecated');
CREATE TYPE http_method AS ENUM ('GET', 'POST', 'PUT', 'PATCH', 'DELETE');
CREATE TYPE protocol_type AS ENUM ('soap', 'http', 'cli', 'ssh', 'pty');
CREATE TYPE delivery_guarantee AS ENUM ('at_most_once', 'at_least_once', 'exactly_once');
CREATE TYPE artifact_type AS ENUM ('plugin_so', 'config_yaml', 'openapi_json', 'dockerfile', 'test_suite', 'agent_prompt');
CREATE TYPE build_status AS ENUM ('building', 'ready', 'failed');
CREATE TYPE delivery_status AS ENUM ('pending', 'delivered', 'failed', 'dead');
CREATE TYPE sandbox_mode AS ENUM ('mock', 'replay', 'proxy');

-- 项目表
CREATE TABLE projects (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        VARCHAR(255) NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    owner       VARCHAR(255) NOT NULL,
    source_type   source_type NOT NULL,
    source_config JSONB NOT NULL DEFAULT '{}',
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 契约表
CREATE TABLE contracts (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id      UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    version         VARCHAR(50) NOT NULL,
    status          contract_status NOT NULL DEFAULT 'draft',
    original_schema TEXT NOT NULL,
    parsed_model    JSONB NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (project_id, version)
);

-- 后端绑定表
CREATE TABLE backend_bindings (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    protocol                protocol_type NOT NULL,
    endpoint_config         JSONB NOT NULL DEFAULT '{}',
    connection_pool_config  JSONB NOT NULL DEFAULT '{"max_connections": 100, "idle_timeout_ms": 30000, "max_lifetime_ms": 300000}',
    circuit_breaker_config  JSONB NOT NULL DEFAULT '{"error_threshold_percent": 50, "window_duration_ms": 30000, "open_duration_ms": 60000, "half_open_max_requests": 3}',
    rate_limit_config       JSONB NOT NULL DEFAULT '{"requests_per_second": 1000, "burst_size": 100}',
    retry_config            JSONB NOT NULL DEFAULT '{"max_retries": 3, "base_delay_ms": 1000, "max_delay_ms": 30000}',
    timeout_ms              BIGINT NOT NULL DEFAULT 30000,
    auth_mapping            JSONB NOT NULL DEFAULT '{}'
);

-- 路由表
CREATE TABLE routes (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    contract_id         UUID NOT NULL REFERENCES contracts(id) ON DELETE CASCADE,
    method              http_method NOT NULL,
    path                VARCHAR(1024) NOT NULL,
    request_schema      JSONB NOT NULL DEFAULT '{}',
    response_schema     JSONB NOT NULL DEFAULT '{}',
    transform_rules     JSONB NOT NULL DEFAULT '{}',
    backend_binding_id  UUID NOT NULL REFERENCES backend_bindings(id),
    delivery_guarantee  delivery_guarantee NOT NULL DEFAULT 'at_most_once',
    enabled             BOOLEAN NOT NULL DEFAULT true,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 生成产物表
CREATE TABLE artifacts (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    contract_id     UUID NOT NULL REFERENCES contracts(id) ON DELETE CASCADE,
    artifact_type   artifact_type NOT NULL,
    content_hash    VARCHAR(64) NOT NULL,
    storage_path    VARCHAR(1024) NOT NULL,
    build_status    build_status NOT NULL DEFAULT 'building',
    build_log       TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 投递记录表（补偿引擎核心）
CREATE TABLE delivery_records (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    route_id          UUID NOT NULL REFERENCES routes(id),
    trace_id          VARCHAR(64) NOT NULL,
    idempotency_key   VARCHAR(255),
    request_payload   JSONB NOT NULL,
    response_payload  JSONB,
    status            delivery_status NOT NULL DEFAULT 'pending',
    retry_count       INT NOT NULL DEFAULT 0,
    next_retry_at     TIMESTAMPTZ,
    error_message     TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 幂等键表
CREATE TABLE idempotency_keys (
    idempotency_key VARCHAR(255) PRIMARY KEY,
    route_id        UUID NOT NULL REFERENCES routes(id),
    status          VARCHAR(20) NOT NULL DEFAULT 'pending',
    response_hash   VARCHAR(64),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 沙箱会话表
CREATE TABLE sandbox_sessions (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id  UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    tenant_id   VARCHAR(255) NOT NULL,
    mode        sandbox_mode NOT NULL,
    config      JSONB NOT NULL DEFAULT '{}',
    expires_at  TIMESTAMPTZ NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 录制交互表
CREATE TABLE recorded_interactions (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    session_id  UUID NOT NULL REFERENCES sandbox_sessions(id) ON DELETE CASCADE,
    route_id    UUID NOT NULL REFERENCES routes(id),
    request     JSONB NOT NULL,
    response    JSONB NOT NULL,
    duration_ms INT NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 索引：按使用频率和查询模式创建
CREATE INDEX idx_contracts_project_id ON contracts(project_id);
CREATE INDEX idx_routes_contract_id ON routes(contract_id);
CREATE INDEX idx_routes_enabled ON routes(enabled) WHERE enabled = true;
CREATE INDEX idx_artifacts_contract_id ON artifacts(contract_id);
CREATE INDEX idx_delivery_records_status ON delivery_records(status) WHERE status IN ('pending', 'failed');
CREATE INDEX idx_delivery_records_next_retry ON delivery_records(next_retry_at) WHERE status = 'failed' AND next_retry_at IS NOT NULL;
CREATE INDEX idx_delivery_records_route_id ON delivery_records(route_id);
CREATE INDEX idx_sandbox_sessions_project ON sandbox_sessions(project_id);
CREATE INDEX idx_recorded_interactions_session ON recorded_interactions(session_id);

-- updated_at 自动更新触发器
CREATE OR REPLACE FUNCTION update_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_projects_updated_at BEFORE UPDATE ON projects FOR EACH ROW EXECUTE FUNCTION update_updated_at();
CREATE TRIGGER trg_contracts_updated_at BEFORE UPDATE ON contracts FOR EACH ROW EXECUTE FUNCTION update_updated_at();
CREATE TRIGGER trg_routes_updated_at BEFORE UPDATE ON routes FOR EACH ROW EXECUTE FUNCTION update_updated_at();
CREATE TRIGGER trg_delivery_records_updated_at BEFORE UPDATE ON delivery_records FOR EACH ROW EXECUTE FUNCTION update_updated_at();
```

- [ ] **Step 2: 实现 repo.rs — MetadataRepo trait**

```rust
use api_anything_common::error::AppError;
use api_anything_common::models::*;
use uuid::Uuid;

/// 元数据仓库的 trait 定义，所有子系统通过此接口访问元数据
pub trait MetadataRepo: Send + Sync {
    // --- Project ---
    async fn create_project(&self, name: &str, description: &str, owner: &str, source_type: SourceType) -> Result<Project, AppError>;
    async fn get_project(&self, id: Uuid) -> Result<Project, AppError>;
    async fn list_projects(&self) -> Result<Vec<Project>, AppError>;
    async fn delete_project(&self, id: Uuid) -> Result<(), AppError>;
}
```

- [ ] **Step 3: 实现 pg.rs — PostgreSQL 实现**

```rust
use crate::repo::MetadataRepo;
use api_anything_common::error::AppError;
use api_anything_common::models::*;
use sqlx::PgPool;
use uuid::Uuid;

pub struct PgMetadataRepo {
    pool: PgPool,
}

impl PgMetadataRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 执行数据库迁移（使用 sqlx 版本化迁移）
    pub async fn run_migrations(&self) -> Result<(), sqlx::Error> {
        sqlx::migrate!("src/migrations")
            .run(&self.pool)
            .await?;
        Ok(())
    }
}

impl MetadataRepo for PgMetadataRepo {
    async fn create_project(
        &self,
        name: &str,
        description: &str,
        owner: &str,
        source_type: SourceType,
    ) -> Result<Project, AppError> {
        let project = sqlx::query_as!(
            Project,
            r#"
            INSERT INTO projects (name, description, owner, source_type)
            VALUES ($1, $2, $3, $4)
            RETURNING id, name, description, owner, source_type AS "source_type: SourceType", created_at, updated_at
            "#,
            name,
            description,
            owner,
            source_type as SourceType,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(project)
    }

    async fn get_project(&self, id: Uuid) -> Result<Project, AppError> {
        let project = sqlx::query_as!(
            Project,
            r#"
            SELECT id, name, description, owner, source_type AS "source_type: SourceType", created_at, updated_at
            FROM projects WHERE id = $1
            "#,
            id,
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Project {id} not found")))?;
        Ok(project)
    }

    async fn list_projects(&self) -> Result<Vec<Project>, AppError> {
        let projects = sqlx::query_as!(
            Project,
            r#"
            SELECT id, name, description, owner, source_type AS "source_type: SourceType", created_at, updated_at
            FROM projects ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(projects)
    }

    async fn delete_project(&self, id: Uuid) -> Result<(), AppError> {
        let result = sqlx::query!("DELETE FROM projects WHERE id = $1", id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::NotFound(format!("Project {id} not found")));
        }
        Ok(())
    }
}
```

- [ ] **Step 4: 更新 lib.rs**

```rust
pub mod pg;
pub mod repo;

pub use pg::PgMetadataRepo;
pub use repo::MetadataRepo;
```

- [ ] **Step 5: 验证编译**

Run: `cargo check -p api-anything-metadata`
Expected: 编译成功

- [ ] **Step 6: Commit**

```bash
git add crates/metadata/
git commit -m "feat(metadata): add initial PostgreSQL schema, MetadataRepo trait, and PG implementation"
```

---

### Task 5: Platform API — Axum 骨架 + 健康检查

**Files:**
- Create: `crates/platform-api/src/state.rs`
- Create: `crates/platform-api/src/routes/mod.rs`
- Create: `crates/platform-api/src/routes/health.rs`
- Create: `crates/platform-api/src/middleware/mod.rs`
- Create: `crates/platform-api/src/middleware/tracing_mw.rs`
- Create: `crates/platform-api/src/middleware/error_handler.rs`
- Modify: `crates/platform-api/src/main.rs`

- [ ] **Step 1: 编写健康检查测试**

创建 `crates/platform-api/tests/health_test.rs`：

```rust
use axum::http::StatusCode;
use axum_test::TestServer;

mod common;

#[tokio::test]
async fn health_check_returns_ok() {
    let server = common::test_server().await;
    let response = server.get("/health").await;
    response.assert_status(StatusCode::OK);

    let body: serde_json::Value = response.json();
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn health_ready_returns_ok_when_db_connected() {
    let server = common::test_server().await;
    let response = server.get("/health/ready").await;
    response.assert_status(StatusCode::OK);
}
```

创建 `crates/platform-api/tests/common/mod.rs`：

```rust
use api_anything_platform_api::build_app;
use axum_test::TestServer;
use sqlx::PgPool;

pub async fn test_server() -> TestServer {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://api_anything:api_anything@localhost:5432/api_anything".to_string());
    let pool = PgPool::connect(&database_url).await.expect("Failed to connect to test DB");
    let app = build_app(pool);
    TestServer::new(app).unwrap()
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p api-anything-platform-api`
Expected: FAIL — `build_app` 未定义

- [ ] **Step 3: 实现 state.rs**

```rust
use sqlx::PgPool;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
}
```

- [ ] **Step 4: 实现 routes/health.rs**

```rust
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use crate::state::AppState;

/// 基础健康检查：服务存活
pub async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok" }))
}

/// 就绪检查：数据库连接可用
pub async fn ready(State(state): State<AppState>) -> impl IntoResponse {
    match sqlx::query("SELECT 1").execute(&state.db).await {
        Ok(_) => (StatusCode::OK, Json(json!({ "status": "ready", "db": "connected" }))).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "Database readiness check failed");
            (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "status": "not_ready", "db": "disconnected" }))).into_response()
        }
    }
}
```

- [ ] **Step 5: 实现 routes/mod.rs**

```rust
pub mod health;
```

- [ ] **Step 6: 实现 middleware/tracing_mw.rs**

```rust
use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::runtime;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// 初始化 tracing + OpenTelemetry
pub fn init_tracing(otel_endpoint: &str) {
    let otlp_exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(otel_endpoint)
        .build()
        .expect("Failed to build OTLP exporter");

    let tracer_provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_batch_exporter(otlp_exporter, runtime::Tokio)
        .build();

    let tracer = tracer_provider.tracer("api-anything");
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer().json())
        .with(otel_layer)
        .init();
}
```

- [ ] **Step 7: 实现 middleware/error_handler.rs**

```rust
use axum::http::StatusCode;
use axum::response::IntoResponse;
use api_anything_common::error::ProblemDetail;

/// 处理 404 路由未匹配的兜底
pub async fn fallback() -> impl IntoResponse {
    ProblemDetail {
        error_type: "about:blank".to_string(),
        title: "Not Found".to_string(),
        status: 404,
        detail: Some("The requested resource was not found".to_string()),
        instance: None,
    }
}
```

- [ ] **Step 8: 实现 middleware/mod.rs**

```rust
pub mod error_handler;
pub mod tracing_mw;
```

- [ ] **Step 9: 重写 main.rs 并导出 build_app**

将 `crates/platform-api/src/main.rs` 拆分为 `lib.rs` + `main.rs`。

创建 `crates/platform-api/src/lib.rs`：

```rust
pub mod middleware;
pub mod routes;
pub mod state;

use axum::{routing::get, Router};
use sqlx::PgPool;
use state::AppState;
use tower_http::trace::TraceLayer;

pub fn build_app(pool: PgPool) -> Router {
    let state = AppState { db: pool };

    Router::new()
        .route("/health", get(routes::health::health))
        .route("/health/ready", get(routes::health::ready))
        .fallback(middleware::error_handler::fallback)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
```

更新 `crates/platform-api/src/main.rs`：

```rust
use api_anything_common::config::AppConfig;
use api_anything_metadata::PgMetadataRepo;
use api_anything_platform_api::build_app;
use api_anything_platform_api::middleware::tracing_mw;
use sqlx::PgPool;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let config = AppConfig::from_env();

    tracing_mw::init_tracing(&config.otel_endpoint);
    tracing::info!("Starting API-Anything Platform API");

    let pool = PgPool::connect(&config.database_url).await?;

    // 运行数据库迁移
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await?;
    tracing::info!("Database migrations completed");

    let app = build_app(pool);
    let addr = format!("{}:{}", config.api_host, config.api_port);
    let listener = TcpListener::bind(&addr).await?;
    tracing::info!("Listening on {addr}");

    axum::serve(listener, app).await?;
    Ok(())
}
```

- [ ] **Step 10: 验证编译**

Run: `cargo check -p api-anything-platform-api`
Expected: 编译成功

- [ ] **Step 11: 启动 Docker 服务并运行测试**

确保 Docker Compose 服务运行中：
Run: `cd docker && docker compose up -d postgres`

运行测试：
Run: `cargo test -p api-anything-platform-api`
Expected: 2 tests PASS

- [ ] **Step 12: Commit**

```bash
git add crates/platform-api/
git commit -m "feat(platform-api): add Axum server skeleton with health checks, OTel tracing, and RFC 7807 error handling"
```

---

### Task 6: Platform API — Project CRUD 端点

**Files:**
- Create: `crates/platform-api/src/routes/projects.rs`
- Modify: `crates/platform-api/src/routes/mod.rs`
- Modify: `crates/platform-api/src/state.rs`
- Modify: `crates/platform-api/src/lib.rs`
- Test: `crates/platform-api/tests/projects_test.rs`

- [ ] **Step 1: 编写 Project CRUD 测试**

创建 `crates/platform-api/tests/projects_test.rs`：

```rust
use axum::http::StatusCode;
use axum_test::TestServer;
use serde_json::json;

mod common;

#[tokio::test]
async fn create_project_returns_201() {
    let server = common::test_server().await;
    let response = server
        .post("/api/v1/projects")
        .json(&json!({
            "name": format!("test-soap-{}", uuid::Uuid::new_v4()),
            "description": "Test SOAP service",
            "owner": "team-test",
            "source_type": "wsdl"
        }))
        .await;
    response.assert_status(StatusCode::CREATED);

    let body: serde_json::Value = response.json();
    assert_eq!(body["name"], "test-soap-service");
    assert_eq!(body["source_type"], "wsdl");
    assert!(body["id"].is_string());
}

#[tokio::test]
async fn get_project_returns_404_for_unknown_id() {
    let server = common::test_server().await;
    let response = server
        .get("/api/v1/projects/00000000-0000-0000-0000-000000000000")
        .await;
    response.assert_status(StatusCode::NOT_FOUND);

    let body: serde_json::Value = response.json();
    assert_eq!(body["status"], 404);
}

#[tokio::test]
async fn list_projects_returns_empty_array() {
    let server = common::test_server().await;
    let response = server.get("/api/v1/projects").await;
    response.assert_status(StatusCode::OK);

    let body: serde_json::Value = response.json();
    assert!(body.is_array());
}

#[tokio::test]
async fn create_then_get_project() {
    let server = common::test_server().await;

    let create_resp = server
        .post("/api/v1/projects")
        .json(&json!({
            "name": format!("roundtrip-{}", uuid::Uuid::new_v4()),
            "description": "Roundtrip test",
            "owner": "team-test",
            "source_type": "cli"
        }))
        .await;
    let created: serde_json::Value = create_resp.json();
    let id = created["id"].as_str().unwrap();

    let get_resp = server
        .get(&format!("/api/v1/projects/{id}"))
        .await;
    get_resp.assert_status(StatusCode::OK);
    let fetched: serde_json::Value = get_resp.json();
    assert_eq!(fetched["name"], "roundtrip-test");
    assert_eq!(fetched["source_type"], "cli");
}

#[tokio::test]
async fn delete_project_returns_204() {
    let server = common::test_server().await;

    let create_resp = server
        .post("/api/v1/projects")
        .json(&json!({
            "name": format!("to-delete-{}", uuid::Uuid::new_v4()),
            "description": "Will be deleted",
            "owner": "team-test",
            "source_type": "ssh"
        }))
        .await;
    let created: serde_json::Value = create_resp.json();
    let id = created["id"].as_str().unwrap();

    let del_resp = server
        .delete(&format!("/api/v1/projects/{id}"))
        .await;
    del_resp.assert_status(StatusCode::NO_CONTENT);

    // 确认已删除
    let get_resp = server
        .get(&format!("/api/v1/projects/{id}"))
        .await;
    get_resp.assert_status(StatusCode::NOT_FOUND);
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p api-anything-platform-api -- projects`
Expected: FAIL — 路由未注册

- [ ] **Step 3: 更新 state.rs，加入 MetadataRepo**

```rust
use api_anything_metadata::PgMetadataRepo;
use sqlx::PgPool;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub repo: Arc<PgMetadataRepo>,
}
```

- [ ] **Step 4: 实现 routes/projects.rs**

```rust
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use api_anything_common::error::AppError;
use api_anything_common::models::SourceType;
use api_anything_metadata::MetadataRepo;
use serde::Deserialize;
use uuid::Uuid;

use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct CreateProjectRequest {
    pub name: String,
    pub description: String,
    pub owner: String,
    pub source_type: SourceType,
}

pub async fn create_project(
    State(state): State<AppState>,
    Json(req): Json<CreateProjectRequest>,
) -> Result<impl IntoResponse, AppError> {
    let project = state.repo.create_project(&req.name, &req.description, &req.owner, req.source_type).await?;
    Ok((StatusCode::CREATED, Json(project)))
}

pub async fn get_project(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let project = state.repo.get_project(id).await?;
    Ok(Json(project))
}

pub async fn list_projects(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let projects = state.repo.list_projects().await?;
    Ok(Json(projects))
}

pub async fn delete_project(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    state.repo.delete_project(id).await?;
    Ok(StatusCode::NO_CONTENT)
}
```

- [ ] **Step 5: 更新 routes/mod.rs**

```rust
pub mod health;
pub mod projects;
```

- [ ] **Step 6: 更新 lib.rs 注册 Project 路由**

```rust
pub mod middleware;
pub mod routes;
pub mod state;

use axum::{routing::{get, post, delete}, Router};
use api_anything_metadata::PgMetadataRepo;
use sqlx::PgPool;
use state::AppState;
use std::sync::Arc;
use tower_http::trace::TraceLayer;

pub fn build_app(pool: PgPool) -> Router {
    let repo = Arc::new(PgMetadataRepo::new(pool.clone()));
    let state = AppState { db: pool, repo };

    Router::new()
        // Health
        .route("/health", get(routes::health::health))
        .route("/health/ready", get(routes::health::ready))
        // Projects CRUD
        .route("/api/v1/projects", post(routes::projects::create_project).get(routes::projects::list_projects))
        .route("/api/v1/projects/{id}", get(routes::projects::get_project).delete(routes::projects::delete_project))
        // Fallback
        .fallback(middleware::error_handler::fallback)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
```

更新 `tests/common/mod.rs` 适配新的 `build_app`：

```rust
use api_anything_platform_api::build_app;
use api_anything_metadata::PgMetadataRepo;
use axum_test::TestServer;
use sqlx::PgPool;

pub async fn test_server() -> TestServer {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://api_anything:api_anything@localhost:5432/api_anything".to_string());
    let pool = PgPool::connect(&database_url).await.expect("Failed to connect to test DB");

    // 运行迁移确保 schema 存在
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.expect("Failed to run migrations");

    let app = build_app(pool);
    TestServer::new(app).unwrap()
}
```

- [ ] **Step 7: 运行测试确认通过**

Run: `cargo test -p api-anything-platform-api`
Expected: 所有 tests PASS（health + projects）

- [ ] **Step 8: Commit**

```bash
git add crates/platform-api/ crates/metadata/
git commit -m "feat(platform-api): add Project CRUD endpoints with PostgreSQL persistence"
```

---

### Task 7: Kafka Topic 初始化脚本

**Files:**
- Create: `docker/init-kafka-topics.sh`

- [ ] **Step 1: 创建 Kafka topic 初始化脚本**

```bash
#!/usr/bin/env bash
set -euo pipefail

BOOTSTRAP="localhost:9092"

# 等待 Kafka 就绪
echo "Waiting for Kafka..."
until kafka-topics --bootstrap-server "$BOOTSTRAP" --list > /dev/null 2>&1; do
    sleep 2
done
echo "Kafka is ready."

# 创建 topics
TOPICS=(
    "route.updated"
    "delivery-events"
    "push-events"
    "generation.completed"
)

for topic in "${TOPICS[@]}"; do
    kafka-topics --bootstrap-server "$BOOTSTRAP" \
        --create --if-not-exists \
        --topic "$topic" \
        --partitions 6 \
        --replication-factor 1
    echo "Created topic: $topic"
done

echo "All Kafka topics initialized."
```

- [ ] **Step 2: 赋予执行权限并测试**

Run: `chmod +x docker/init-kafka-topics.sh`

Run: `docker compose -f docker/docker-compose.yml exec kafka bash -c '/opt/kafka/bin/kafka-topics.sh --bootstrap-server localhost:9092 --list'`
Expected: 能连接 Kafka（topics 可能为空）

- [ ] **Step 3: Commit**

```bash
git add docker/init-kafka-topics.sh
git commit -m "infra: add Kafka topic initialization script"
```

---

### Task 8: CI/CD Pipeline (GitHub Actions)

**Files:**
- Create: `.github/workflows/ci.yml`

- [ ] **Step 1: 创建 CI workflow**

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

env:
  CARGO_TERM_COLOR: always
  RUST_BACKTRACE: 1
  DATABASE_URL: postgres://api_anything:api_anything@localhost:5432/api_anything

jobs:
  check:
    name: Check & Lint
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.82.0
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --all -- --check
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo check --workspace

  test:
    name: Test
    runs-on: ubuntu-latest
    services:
      postgres:
        image: postgres:16-alpine
        env:
          POSTGRES_USER: api_anything
          POSTGRES_PASSWORD: api_anything
          POSTGRES_DB: api_anything
        ports:
          - 5432:5432
        options: >-
          --health-cmd "pg_isready -U api_anything"
          --health-interval 5s
          --health-timeout 5s
          --health-retries 10
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.82.0
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --workspace
```

- [ ] **Step 2: Commit**

```bash
git add .github/
git commit -m "ci: add GitHub Actions workflow with check, lint, and test jobs"
```

---

### Task 9: 端到端验证

**Files:** 无新文件

- [ ] **Step 1: 启动完整 Docker 环境**

Run: `cd docker && docker compose up -d`
Expected: 所有服务启动（postgres, zookeeper, kafka, otel-collector, tempo, prometheus, loki, grafana）

- [ ] **Step 2: 运行数据库迁移**

Run: `cargo run -p api-anything-platform-api`
Expected: 服务启动成功，日志输出 "Database migrations completed" 和 "Listening on 0.0.0.0:8080"

Ctrl+C 停止。

- [ ] **Step 3: 运行全量测试**

Run: `cargo test --workspace`
Expected: 所有测试通过

- [ ] **Step 4: 验证 API 端到端**

启动服务后，在另一个终端：

```bash
# 健康检查
curl -s http://localhost:8080/health | jq .
# Expected: {"status":"ok"}

# 就绪检查
curl -s http://localhost:8080/health/ready | jq .
# Expected: {"status":"ready","db":"connected"}

# 创建项目
curl -s -X POST http://localhost:8080/api/v1/projects \
  -H "Content-Type: application/json" \
  -d '{"name":"demo-soap","description":"Demo SOAP service","owner":"team-test","source_type":"wsdl"}' | jq .

# 列出项目
curl -s http://localhost:8080/api/v1/projects | jq .

# 404 测试
curl -s http://localhost:8080/nonexistent | jq .
# Expected: RFC 7807 格式 404
```

- [ ] **Step 5: 验证 Grafana 数据源**

浏览器访问 `http://localhost:3000`，登录 admin/admin。
检查：Configuration → Data Sources → Prometheus, Tempo, Loki 均显示为 configured。

- [ ] **Step 6: 验证 CLI 骨架**

Run: `cargo run -p api-anything-cli -- --help`
Expected: 显示帮助信息，包含 `generate` 子命令

Run: `cargo run -p api-anything-cli -- generate --source test.wsdl`
Expected: 显示 "not yet implemented"

- [ ] **Step 7: Commit（如有修复）**

如果端到端验证中发现并修复了问题：

```bash
git add -A
git commit -m "fix: address issues found during e2e validation"
```

---

## Summary

| Task | 组件 | 产出 |
|------|------|------|
| 1 | Rust Workspace | 4 crate 骨架 + workspace 配置 |
| 2 | Common Crate | 领域模型 + RFC 7807 错误 + 配置加载 |
| 3 | Docker Compose | PG + Kafka + OTel 全栈本地环境 |
| 4 | Metadata Crate | DDL 迁移 + MetadataRepo trait + PG 实现 |
| 5 | Platform API 骨架 | Axum server + 健康检查 + OTel tracing |
| 6 | Project CRUD | /api/v1/projects 完整 CRUD + 测试 |
| 7 | Kafka Topics | 初始化脚本（4 个核心 topic） |
| 8 | CI/CD | GitHub Actions (check + lint + test) |
| 9 | 端到端验证 | 全链路验证所有组件集成 |

**Phase 0 验收标准：** workspace 编译通过、全量测试通过、Docker 一键启动全部基础设施、API 健康检查和 Project CRUD 正常工作、Grafana 数据源配置完毕、CI pipeline 可运行。
