# API-Anything 环境部署指南

本文档覆盖从本地开发到生产环境的完整部署流程。所有命令均基于项目实际配置文件编写，可直接运行。

---

## 目录

1. [前置依赖](#1-前置依赖)
2. [本地开发环境部署](#2-本地开发环境部署)
3. [测试环境部署](#3-测试环境部署)
4. [预发布/Staging 环境](#4-预发布staging-环境)
5. [生产环境部署](#5-生产环境部署)
6. [运维手册](#6-运维手册)
7. [常用命令速查表](#7-常用命令速查表)

---

## 1. 前置依赖

### 1.1 Rust 工具链

项目要求 Rust 最低版本 **1.82**（见 `Cargo.toml` 中 `rust-version = "1.82"`），edition 为 2021。

```bash
# 安装 rustup（如尚未安装）
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 确保工具链版本满足要求
rustup update stable
rustc --version   # 需 >= 1.82

# 安装开发所需组件
rustup component add rustfmt clippy
```

### 1.2 Node.js + npm

前端基于 Vite 8 + React 19 + TypeScript 5.9，需要 Node.js 20+。

```bash
# 推荐使用 fnm 或 nvm 管理版本
node --version   # 需 >= 20.x
npm --version    # 需 >= 10.x
```

### 1.3 PostgreSQL 16+

docker-compose 中使用的镜像为 `postgres:16-alpine`，本地直连或容器化运行均可。

```bash
# macOS (Homebrew)
brew install postgresql@16

# 或通过容器运行（推荐，见第 2 节）
```

### 1.4 Apache Kafka

docker-compose 中使用 `confluentinc/cp-kafka:7.7.0` + `confluentinc/cp-zookeeper:7.7.0`。本地开发推荐通过容器启动。

### 1.5 Podman 或 Docker（容器运行时）

项目 docker-compose 文件位于 `docker/docker-compose.yml`，Podman Compose 和 Docker Compose 均可使用。

```bash
# macOS - Podman
brew install podman podman-compose
podman machine init
podman machine start

# 或 Docker Desktop
brew install --cask docker
```

### 1.6 系统依赖

Rust 编译部分 crate 需要以下系统库：

| 依赖 | 用途 | 安装方式 (macOS) | 安装方式 (Ubuntu) |
|------|------|-----------------|------------------|
| **cmake** | 编译 `rdkafka`（Kafka 客户端，使用 `cmake-build` feature） | `brew install cmake` | `apt install cmake` |
| **openssl** | TLS 支持（sqlx 使用 rustls，但部分系统仍需） | `brew install openssl` | `apt install libssl-dev pkg-config` |
| **librdkafka** | Kafka C 客户端库（可选，默认 cmake-build 从源码编译） | `brew install librdkafka` | `apt install librdkafka-dev` |
| **psql** | 运行 SQL 迁移脚本 | `brew install libpq` | `apt install postgresql-client` |

```bash
# macOS 一键安装
brew install cmake openssl librdkafka libpq

# Ubuntu / Debian
sudo apt update && sudo apt install -y cmake libssl-dev pkg-config librdkafka-dev postgresql-client build-essential
```

---

## 2. 本地开发环境部署

### 2.1 克隆仓库

```bash
git clone https://github.com/<org>/api-anything.git
cd api-anything
```

### 2.2 环境变量配置

项目根目录提供了 `.env.example`，复制后按需修改：

```bash
cp .env.example .env
```

默认配置内容（所有配置项均有本地开发默认值，不修改也可运行）：

| 环境变量 | 默认值 | 说明 |
|---------|--------|------|
| `DATABASE_URL` | `postgres://api_anything:api_anything@localhost:5432/api_anything` | PostgreSQL 连接串 |
| `KAFKA_BROKERS` | `localhost:9092` | Kafka broker 地址 |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | `http://localhost:4317` | OTel Collector gRPC 端点 |
| `API_HOST` | `0.0.0.0` | 服务监听地址 |
| `API_PORT` | `8080` | 服务监听端口 |
| `RUST_LOG` | `api_anything=debug,tower_http=debug` | 日志级别 |
| `EVENT_BUS_TYPE` | `pg` | 事件总线类型：`pg`（PostgreSQL）或 `kafka` |
| `ALERT_WEBHOOK_URL` | 无 | 告警通知推送地址（可选） |
| `ALERT_WEBHOOK_TYPE` | 无 | 告警推送格式：`slack` 或 `dingtalk`（可选） |
| `PLUGIN_DIR` | `./plugins` | 自定义协议插件目录路径 |

### 2.3 使用 Podman Compose 启动基础设施

docker-compose 文件包含完整的开发基础设施栈：

| 服务 | 镜像 | 端口 | 用途 |
|------|------|------|------|
| postgres | `postgres:16-alpine` | 5432 | 主数据库 |
| zookeeper | `confluentinc/cp-zookeeper:7.7.0` | 2181 | Kafka 依赖 |
| kafka | `confluentinc/cp-kafka:7.7.0` | 9092 | 消息队列 |
| otel-collector | `otel/opentelemetry-collector-contrib:0.114.0` | 4317 (gRPC), 4318 (HTTP) | 遥测数据收集 |
| tempo | `grafana/tempo:2.6.1` | 3200 | 链路追踪存储 |
| prometheus | `prom/prometheus:v2.55.1` | 9090 | 指标存储 |
| loki | `grafana/loki:3.3.2` | 3100 | 日志聚合 |
| grafana | `grafana/grafana:11.4.0` | 3000 | 可观测性仪表盘 |

```bash
# 启动全部基础设施（不含 web 前端容器）
cd docker
podman-compose up -d postgres zookeeper kafka otel-collector tempo prometheus loki grafana

# 或使用 Docker Compose
docker compose up -d postgres zookeeper kafka otel-collector tempo prometheus loki grafana

# 等待 PostgreSQL 就绪
podman-compose exec postgres pg_isready -U api_anything -d api_anything
```

> **提示**：`web` 服务是 nginx 容器，用于生产环境前后端分离部署。本地开发时前端通过 Vite dev server 运行，后端通过 `platform-api` 的 `fallback_service` 托管静态文件，无需启动 web 容器。

### 2.4 运行数据库迁移

迁移脚本位于 `crates/metadata/src/migrations/20260321000000_initial_schema.sql`，采用幂等设计（`IF NOT EXISTS` + 先删后建触发器），可重复执行。

```bash
psql "$DATABASE_URL" -f crates/metadata/src/migrations/20260321000000_initial_schema.sql
```

应用启动时也会自动调用 `repo.run_migrations().await`，但建议首次手动执行以确认连通性。

该迁移脚本会创建以下数据表：

- `projects` — 项目定义（WSDL/OData/CLI/SSH/PTY 数据源）
- `contracts` — API 合约版本
- `backend_bindings` — 后端协议绑定（连接池、熔断器、限流、重试配置）
- `routes` — 路由规则（请求/响应 schema、转换规则、投递保证）
- `artifacts` — 构建产物
- `delivery_records` — 投递记录（重试追踪）
- `idempotency_keys` — 幂等键
- `sandbox_sessions` — 沙箱会话
- `recorded_interactions` — 沙箱录制交互

### 2.5 编译和启动各服务

项目为 Cargo workspace，包含 8 个 crate：

| Crate | 类型 | 说明 |
|-------|------|------|
| `common` | lib | 通用类型、配置、错误定义 |
| `metadata` | lib | 数据库访问层（sqlx + PostgreSQL） |
| `generator` | lib | 从 WSDL/OData 等源生成 API 合约 |
| `gateway` | lib | 动态路由器、后端分发器、协议适配 |
| `sandbox` | lib | 沙箱模式（mock/replay/proxy） |
| `compensation` | lib | 补偿系统、死信队列、重试 worker |
| `platform-api` | **bin** | 主服务（Axum HTTP 服务器） |
| `cli` | **bin** | 命令行工具 |

```bash
# 编译整个 workspace
cargo build --workspace

# 启动主服务（platform-api）
cargo run -p api-anything-platform-api

# 服务启动后会输出：
#   Starting API-Anything Platform API
#   Database migrations completed
#   Gateway routes loaded
#   Retry worker started
#   Listening on 0.0.0.0:8080
```

### 2.6 验证服务健康状态

服务提供两个健康检查端点：

```bash
# Liveness 探针 — 进程存活即返回 200，不依赖外部服务
curl http://localhost:8080/health
# => {"status":"ok"}

# Readiness 探针 — 检测数据库连通性，确保可处理流量
curl http://localhost:8080/health/ready
# => {"status":"ready","db":"connected"}
```

其他可验证的端点：

```bash
# API 文档（Swagger UI）
curl http://localhost:8080/api/v1/docs

# OpenAPI JSON 规范
curl http://localhost:8080/api/v1/docs/openapi.json

# Agent 提示词
curl http://localhost:8080/api/v1/docs/agent-prompt
```

### 2.7 前端开发模式启动

前端使用 Vite 8 + React 19 + Tailwind CSS 4 + TypeScript 5.9。

```bash
cd web
npm install
npm run dev
```

Vite dev server 默认监听 `http://localhost:5173`，并通过 `vite.config.ts` 中的 proxy 配置将 API 请求代理到后端：

| 前缀 | 代理目标 | 说明 |
|------|---------|------|
| `/api` | `http://localhost:8080` | 平台 API |
| `/gw` | `http://localhost:8080` | 网关路由 |
| `/sandbox` | `http://localhost:8080` | 沙箱路由 |

### 2.8 访问 Grafana 仪表盘

Grafana 默认访问地址：`http://localhost:3000`

- 用户名：`admin`
- 密码：`admin`
- 匿名访问已启用（Viewer 权限）

数据源已通过 provisioning 自动配置（`docker/grafana/provisioning/datasources/datasources.yml`）：

| 数据源 | 地址 | 用途 |
|--------|------|------|
| Prometheus（默认） | `http://prometheus:9090` | 指标查询 |
| Tempo | `http://tempo:3200` | 链路追踪，支持 trace-to-logs 跳转到 Loki |
| Loki | `http://loki:3100` | 日志查询，支持从 `trace_id` 跳转到 Tempo |

---

## 3. 测试环境部署

### 3.1 CI/CD Pipeline（GitHub Actions）

CI 配置位于 `.github/workflows/ci.yml`，在 `push` 和 `pull_request` 到 `main` 分支时触发，包含两个 job：

**Job 1: Check & Lint**

```yaml
steps:
  - cargo fmt --all -- --check     # 格式检查
  - cargo clippy --workspace --all-targets -- -D warnings  # Lint
  - cargo check --workspace        # 编译检查
```

**Job 2: Test**

使用 GitHub Actions 的 `services` 启动 PostgreSQL 16 容器：

```yaml
services:
  postgres:
    image: postgres:16-alpine
    env:
      POSTGRES_USER: api_anything
      POSTGRES_PASSWORD: api_anything
      POSTGRES_DB: api_anything
    ports:
      - 5432:5432
```

测试步骤：

```yaml
steps:
  - name: Run migrations
    run: psql "$DATABASE_URL" -f crates/metadata/src/migrations/20260321000000_initial_schema.sql
  - run: cargo test --workspace
```

### 3.2 本地运行全量测试

```bash
# 确保 PostgreSQL 已启动且迁移已执行
psql "$DATABASE_URL" -f crates/metadata/src/migrations/20260321000000_initial_schema.sql

# 运行全部测试
cargo test --workspace

# 运行特定 crate 的测试
cargo test -p api-anything-gateway
cargo test -p api-anything-platform-api
cargo test -p api-anything-compensation
```

### 3.3 测试覆盖率报告生成

```bash
# 安装 cargo-llvm-cov
cargo install cargo-llvm-cov

# 生成 HTML 覆盖率报告
cargo llvm-cov --workspace --html --output-dir target/coverage

# 生成 LCOV 格式（适合 CI 上传）
cargo llvm-cov --workspace --lcov --output-path target/coverage/lcov.info

# 在终端直接查看摘要
cargo llvm-cov --workspace
```

### 3.4 前端测试与 Lint

```bash
cd web
npm run lint      # ESLint 检查
npm run build     # TypeScript 类型检查 + 构建验证
```

---

## 4. 预发布/Staging 环境

### 4.1 Kubernetes 部署清单

#### Namespace

```yaml
apiVersion: v1
kind: Namespace
metadata:
  name: api-anything-staging
```

#### ConfigMap

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: api-anything-config
  namespace: api-anything-staging
data:
  API_HOST: "0.0.0.0"
  API_PORT: "8080"
  KAFKA_BROKERS: "kafka.api-anything-staging.svc.cluster.local:9092"
  OTEL_EXPORTER_OTLP_ENDPOINT: "http://otel-collector.api-anything-staging.svc.cluster.local:4317"
  RUST_LOG: "api_anything=info,tower_http=info"
```

#### Secret

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: api-anything-secrets
  namespace: api-anything-staging
type: Opaque
stringData:
  DATABASE_URL: "postgres://api_anything:<password>@postgres.api-anything-staging.svc.cluster.local:5432/api_anything"
```

#### Deployment — platform-api

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: platform-api
  namespace: api-anything-staging
spec:
  replicas: 2
  selector:
    matchLabels:
      app: platform-api
  template:
    metadata:
      labels:
        app: platform-api
    spec:
      containers:
        - name: platform-api
          image: ghcr.io/<org>/api-anything-platform-api:staging
          ports:
            - containerPort: 8080
          envFrom:
            - configMapRef:
                name: api-anything-config
            - secretRef:
                name: api-anything-secrets
          livenessProbe:
            httpGet:
              path: /health
              port: 8080
            initialDelaySeconds: 5
            periodSeconds: 10
          readinessProbe:
            httpGet:
              path: /health/ready
              port: 8080
            initialDelaySeconds: 10
            periodSeconds: 5
          resources:
            requests:
              cpu: 250m
              memory: 256Mi
            limits:
              cpu: "1"
              memory: 512Mi
```

#### Service

```yaml
apiVersion: v1
kind: Service
metadata:
  name: platform-api
  namespace: api-anything-staging
spec:
  selector:
    app: platform-api
  ports:
    - port: 8080
      targetPort: 8080
  type: ClusterIP
```

#### Deployment — web (nginx)

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: web
  namespace: api-anything-staging
spec:
  replicas: 2
  selector:
    matchLabels:
      app: web
  template:
    metadata:
      labels:
        app: web
    spec:
      containers:
        - name: web
          image: ghcr.io/<org>/api-anything-web:staging
          ports:
            - containerPort: 80
          resources:
            requests:
              cpu: 50m
              memory: 32Mi
            limits:
              cpu: 200m
              memory: 64Mi
```

### 4.2 Helm Chart 结构说明

推荐的 Helm Chart 目录结构：

```
helm/api-anything/
  Chart.yaml
  values.yaml
  values-staging.yaml
  values-production.yaml
  templates/
    _helpers.tpl
    namespace.yaml
    configmap.yaml
    secret.yaml
    deployment-platform-api.yaml
    deployment-web.yaml
    service-platform-api.yaml
    service-web.yaml
    ingress.yaml
    hpa.yaml
```

`values-staging.yaml` 示例：

```yaml
replicaCount:
  platformApi: 2
  web: 2

image:
  platformApi:
    repository: ghcr.io/<org>/api-anything-platform-api
    tag: staging
  web:
    repository: ghcr.io/<org>/api-anything-web
    tag: staging

config:
  apiHost: "0.0.0.0"
  apiPort: 8080
  kafkaBrokers: "kafka:9092"
  otelEndpoint: "http://otel-collector:4317"
  rustLog: "api_anything=info,tower_http=info"

resources:
  platformApi:
    requests: { cpu: 250m, memory: 256Mi }
    limits: { cpu: "1", memory: 512Mi }
  web:
    requests: { cpu: 50m, memory: 32Mi }
    limits: { cpu: 200m, memory: 64Mi }
```

### 4.3 环境变量和 Secret 管理

| 方式 | 适用场景 | 说明 |
|------|---------|------|
| Kubernetes Secret | Staging/Production | `DATABASE_URL` 等敏感配置 |
| Kubernetes ConfigMap | Staging/Production | 非敏感配置（`API_PORT`、`RUST_LOG` 等） |
| External Secrets Operator | 生产环境 | 从 Vault / AWS Secrets Manager 同步 |
| `.env` 文件 | 本地开发 | 仅在本地使用，不纳入版本控制 |

### 4.4 OTel Collector 配置

Staging 环境的 OTel Collector 配置与 `docker/otel-collector-config.yml` 保持一致：

- **Receivers**: OTLP gRPC (4317) + HTTP (4318)
- **Processors**: Batch（5s 超时，每批 1024 条）
- **Exporters**:
  - Traces → Tempo（`otlp/tempo`）
  - Metrics → Prometheus Remote Write
  - Logs → Loki

```yaml
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

### 4.5 Grafana 数据源配置

通过 provisioning 自动加载数据源（`docker/grafana/provisioning/datasources/datasources.yml`）：

- **Prometheus**（默认数据源）：指标查询
- **Tempo**：链路追踪，配置了 `tracesToLogsV2`（关联 Loki）和 `tracesToMetrics`（关联 Prometheus）
- **Loki**：日志查询，配置了 `derivedFields` 从日志中的 `trace_id` 提取链路追踪链接

### 4.6 Grafana Dashboard 预置面板

Docker Compose 启动的 Grafana 实例预配置了 API-Anything 专用仪表盘（通过 provisioning 自动加载），包含以下面板：

| 面板 | 数据源 | PromQL / 查询 | 说明 |
|------|--------|--------------|------|
| **QPS** | Prometheus | `rate(http_server_request_duration_seconds_count[1m])` | 网关每秒请求数，按路径分组 |
| **Latency P99** | Prometheus | `histogram_quantile(0.99, rate(http_server_request_duration_seconds_bucket[5m]))` | 第 99 百分位延迟 |
| **Error Rate** | Prometheus | `rate(http_server_request_duration_seconds_count{status=~"5.."}[5m]) / rate(http_server_request_duration_seconds_count[5m])` | 5xx 错误比率 |
| **Circuit Breaker Status** | Prometheus | `circuit_breaker_state` | 各后端绑定的熔断器状态（Closed/Open/HalfOpen） |
| **Backend Latency** | Prometheus | `histogram_quantile(0.95, rate(backend_request_duration_seconds_bucket[5m]))` | 按协议类型分组的后端响应延迟 |

这些面板在 Web 管理平台的 Monitoring 页面（`/monitoring`）中通过 iframe 嵌入展示，无需单独访问 Grafana。

---

## 5. 生产环境部署

### 5.1 容器镜像构建 — 后端（Rust musl 静态编译）

使用多阶段构建，最终产物为 `FROM scratch` 的极小镜像：

```dockerfile
# Stage 1: 编译 — 使用 musl 目标实现完全静态链接
FROM rust:1.82-alpine AS builder
RUN apk add --no-cache musl-dev cmake make gcc g++ openssl-dev openssl-libs-static pkgconfig
WORKDIR /src
COPY . .
RUN cargo build --release --target x86_64-unknown-linux-musl -p api-anything-platform-api

# Stage 2: 运行 — scratch 镜像，仅包含单一二进制文件
FROM scratch
COPY --from=builder /src/target/x86_64-unknown-linux-musl/release/api-anything-platform-api /app
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
EXPOSE 8080
ENTRYPOINT ["/app"]
```

```bash
# 构建镜像
podman build -t api-anything-platform-api:latest -f Dockerfile.platform-api .

# 或 Docker
docker build -t api-anything-platform-api:latest -f Dockerfile.platform-api .
```

### 5.2 容器镜像构建 — 前端

前端已有 `web/Dockerfile`，采用两阶段构建：

1. **Stage 1 (builder)**：`node:20-alpine`，执行 `npm ci` + `npm run build`（TypeScript 编译 + Vite 构建）
2. **Stage 2 (runtime)**：`nginx:alpine`，复制构建产物到 `/usr/share/nginx/html`，使用自定义 `nginx.conf`

```bash
cd web
podman build -t api-anything-web:latest .
```

nginx 配置（`web/nginx.conf`）要点：
- SPA 路由兜底：所有非文件请求回退到 `index.html`，由 React Router 接管
- API 代理：`/api/`、`/gw/`、`/sandbox/`、`/health` 转发到 `platform-api:8080`

> **备选方案**：`platform-api` 内置了 `fallback_service`（`ServeDir` + `ServeFile`），可直接从 `web/dist` 托管前端静态文件，实现单容器部署。适用于小规模场景。

### 5.3 Kubernetes HPA 弹性伸缩

```yaml
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: platform-api-hpa
  namespace: api-anything-production
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: platform-api
  minReplicas: 3
  maxReplicas: 20
  metrics:
    - type: Resource
      resource:
        name: cpu
        target:
          type: Utilization
          averageUtilization: 70
    - type: Resource
      resource:
        name: memory
        target:
          type: Utilization
          averageUtilization: 80
  behavior:
    scaleUp:
      stabilizationWindowSeconds: 30
      policies:
        - type: Pods
          value: 4
          periodSeconds: 60
    scaleDown:
      stabilizationWindowSeconds: 300
      policies:
        - type: Pods
          value: 1
          periodSeconds: 60
```

### 5.4 PostgreSQL 主从配置要点

| 配置项 | 推荐值 | 说明 |
|--------|--------|------|
| `max_connections` | 200 | 根据 `platform-api` 副本数 x 连接池大小估算 |
| `shared_buffers` | 25% 物理内存 | 数据缓存 |
| `wal_level` | `replica` | 支持流复制 |
| `max_wal_senders` | 10 | 从库连接数 |
| `synchronous_commit` | `on` | 生产环境建议同步提交 |

连接池配置（`backend_bindings.connection_pool_config` 默认值）：
- `max_connections`: 100
- `idle_timeout_ms`: 30000
- `max_lifetime_ms`: 300000

### 5.5 Kafka 集群配置要点

| 配置项 | 开发环境 | 生产环境 | 说明 |
|--------|---------|---------|------|
| `KAFKA_OFFSETS_TOPIC_REPLICATION_FACTOR` | 1 | 3 | 偏移量主题副本数 |
| `KAFKA_DEFAULT_REPLICATION_FACTOR` | 1 | 3 | 默认主题副本数 |
| `KAFKA_AUTO_CREATE_TOPICS_ENABLE` | true | false | 生产环境应手动创建主题 |
| Broker 数量 | 1 | >= 3 | 高可用最低要求 |
| `min.insync.replicas` | 1 | 2 | 配合 `acks=all` 确保数据持久性 |

### 5.6 TLS/HTTPS 配置

#### Ingress 配置（cert-manager 自动签发）

```yaml
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: api-anything-ingress
  namespace: api-anything-production
  annotations:
    cert-manager.io/cluster-issuer: letsencrypt-prod
spec:
  tls:
    - hosts:
        - api.example.com
      secretName: api-anything-tls
  rules:
    - host: api.example.com
      http:
        paths:
          - path: /api
            pathType: Prefix
            backend:
              service:
                name: platform-api
                port:
                  number: 8080
          - path: /gw
            pathType: Prefix
            backend:
              service:
                name: platform-api
                port:
                  number: 8080
          - path: /sandbox
            pathType: Prefix
            backend:
              service:
                name: platform-api
                port:
                  number: 8080
          - path: /health
            pathType: Prefix
            backend:
              service:
                name: platform-api
                port:
                  number: 8080
          - path: /
            pathType: Prefix
            backend:
              service:
                name: web
                port:
                  number: 80
```

### 5.7 监控告警配置（Prometheus Alerts）

```yaml
groups:
  - name: api-anything
    rules:
      - alert: PlatformApiDown
        expr: up{job="api-anything"} == 0
        for: 1m
        labels:
          severity: critical
        annotations:
          summary: "platform-api 实例宕机"
          description: "{{ $labels.instance }} 已持续 1 分钟不可达"

      - alert: HighErrorRate
        expr: |
          rate(http_server_request_duration_seconds_count{status=~"5.."}[5m])
          / rate(http_server_request_duration_seconds_count[5m]) > 0.05
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "HTTP 5xx 错误率超过 5%"

      - alert: HighP99Latency
        expr: |
          histogram_quantile(0.99, rate(http_server_request_duration_seconds_bucket[5m])) > 2
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "P99 延迟超过 2 秒"

      - alert: DeadLetterQueueGrowing
        expr: |
          increase(delivery_records_dead_total[1h]) > 10
        for: 10m
        labels:
          severity: warning
        annotations:
          summary: "死信队列 1 小时内新增超过 10 条"

      - alert: DatabaseConnectionPoolExhausted
        expr: |
          sqlx_pool_idle_connections / sqlx_pool_max_connections < 0.1
        for: 5m
        labels:
          severity: critical
        annotations:
          summary: "数据库连接池剩余不足 10%"
```

Prometheus 抓取配置（对应 `docker/prometheus.yml`）：

```yaml
scrape_configs:
  - job_name: "otel-collector"
    static_configs:
      - targets: ["otel-collector:8888"]
  - job_name: "api-anything"
    static_configs:
      - targets: ["platform-api:8080"]
```

### 5.8 日志采集（Loki）

Loki 配置对应 `docker/loki-config.yml`：

- Schema: v13 (TSDB)
- 存储后端: filesystem（生产环境建议替换为 S3/GCS）
- 索引周期: 24h

应用日志通过 OTel Collector 的 logs pipeline 发送到 Loki。`RUST_LOG` 环境变量控制日志级别：

```bash
# 生产环境推荐
RUST_LOG=api_anything=info,tower_http=warn
```

### 5.9 链路追踪（Tempo）

Tempo 配置对应 `docker/tempo-config.yml`：

- 接收协议: OTLP gRPC (4317)
- 存储后端: local（生产环境建议替换为 S3/GCS）
- HTTP API: 3200

应用通过 `tracing-opentelemetry` + `opentelemetry-otlp` 将 trace 数据发送到 OTel Collector，再由 Collector 转发至 Tempo。

Grafana 中已配置 Tempo 数据源的跨源跳转：
- trace → logs：通过 `trace_id` 关联到 Loki
- trace → metrics：关联到 Prometheus

---

## 6. 运维手册

### 6.1 服务健康检查端点

| 端点 | 方法 | 用途 | 成功响应 |
|------|------|------|---------|
| `/health` | GET | Liveness 探针，检测进程存活 | `200 {"status":"ok"}` |
| `/health/ready` | GET | Readiness 探针，检测数据库连通性 | `200 {"status":"ready","db":"connected"}` |

```bash
# 快速检查
curl -sf http://localhost:8080/health && echo "alive" || echo "dead"
curl -sf http://localhost:8080/health/ready && echo "ready" || echo "not ready"
```

### 6.2 日志查看方法

```bash
# 本地开发 — 直接查看 cargo run 终端输出（JSON 格式由 tracing-subscriber 输出）

# 容器环境
podman logs -f <container-name>
docker logs -f <container-name>

# Kubernetes
kubectl logs -f deployment/platform-api -n api-anything-production

# Grafana → Explore → Loki
# 查询示例：
#   {job="api-anything"} |= "error"
#   {job="api-anything"} | json | level="ERROR"
#   {job="api-anything"} | json | trace_id="<trace-id>"
```

### 6.3 常见故障排查

#### 数据库连接失败

```
Database readiness check failed
```

排查步骤：

```bash
# 1. 检查 PostgreSQL 是否运行
podman-compose exec postgres pg_isready -U api_anything -d api_anything

# 2. 验证 DATABASE_URL 连通性
psql "$DATABASE_URL" -c "SELECT 1"

# 3. 检查连接数是否耗尽
psql "$DATABASE_URL" -c "SELECT count(*) FROM pg_stat_activity WHERE datname = 'api_anything'"
```

#### Kafka 连接失败

```bash
# 1. 检查 Kafka broker 状态
podman-compose exec kafka kafka-broker-api-versions --bootstrap-server localhost:9092

# 2. 查看 topic 列表
podman-compose exec kafka kafka-topics --list --bootstrap-server localhost:9092
```

#### OTel Collector 不接收数据

```bash
# 1. 检查 Collector 运行状态
curl http://localhost:8888/metrics  # Collector 自身指标

# 2. 验证 gRPC 端口可达
grpcurl -plaintext localhost:4317 list

# 3. 检查 OTEL_EXPORTER_OTLP_ENDPOINT 配置是否正确
```

#### 路由 404

```bash
# 检查路由表是否已加载（服务日志中应有 "Gateway routes loaded" 及 routes 数量）
# 如果 routes = 0，说明数据库中没有 enabled 的路由配置

psql "$DATABASE_URL" -c "SELECT id, method, path, enabled FROM routes WHERE enabled = true"
```

#### 前端 API 请求失败（开发模式）

确保 Vite dev server 的 proxy 配置正确，且后端在 `localhost:8080` 运行：

```bash
curl http://localhost:8080/api/v1/projects  # 直接请求后端确认可达
```

### 6.4 数据库迁移回滚

迁移脚本采用幂等设计，不提供自动回滚。手动回滚需按以下步骤操作：

```bash
# 1. 查看当前数据库对象
psql "$DATABASE_URL" -c "\dt"   # 查看所有表
psql "$DATABASE_URL" -c "\di"   # 查看所有索引

# 2. 手动回滚示例（按依赖顺序 DROP）
psql "$DATABASE_URL" <<'SQL'
-- 先删除有外键依赖的表
DROP TABLE IF EXISTS recorded_interactions CASCADE;
DROP TABLE IF EXISTS sandbox_sessions CASCADE;
DROP TABLE IF EXISTS idempotency_keys CASCADE;
DROP TABLE IF EXISTS delivery_records CASCADE;
DROP TABLE IF EXISTS artifacts CASCADE;
DROP TABLE IF EXISTS routes CASCADE;
DROP TABLE IF EXISTS backend_bindings CASCADE;
DROP TABLE IF EXISTS contracts CASCADE;
DROP TABLE IF EXISTS projects CASCADE;

-- 删除自定义类型
DROP TYPE IF EXISTS sandbox_mode;
DROP TYPE IF EXISTS delivery_status;
DROP TYPE IF EXISTS build_status;
DROP TYPE IF EXISTS artifact_type;
DROP TYPE IF EXISTS delivery_guarantee;
DROP TYPE IF EXISTS protocol_type;
DROP TYPE IF EXISTS http_method;
DROP TYPE IF EXISTS contract_status;
DROP TYPE IF EXISTS source_type;

-- 删除函数
DROP FUNCTION IF EXISTS update_updated_at();
SQL
```

> **警告**：回滚操作会删除所有数据，仅在开发/测试环境使用。生产环境回滚前务必备份。

### 6.5 灾难恢复

#### 数据库备份与恢复

```bash
# 备份
pg_dump "$DATABASE_URL" --format=custom --file=backup_$(date +%Y%m%d_%H%M%S).dump

# 恢复
pg_restore --dbname="$DATABASE_URL" --clean --if-exists backup_YYYYMMDD_HHMMSS.dump

# 仅恢复 schema（不含数据）
pg_restore --dbname="$DATABASE_URL" --schema-only --clean --if-exists backup.dump
```

#### Kubernetes 环境灾难恢复

```bash
# 1. 确认集群状态
kubectl get nodes
kubectl get pods -n api-anything-production

# 2. 重新部署
kubectl rollout restart deployment/platform-api -n api-anything-production
kubectl rollout restart deployment/web -n api-anything-production

# 3. 检查部署状态
kubectl rollout status deployment/platform-api -n api-anything-production

# 4. 如新版本有问题，回滚到上一版本
kubectl rollout undo deployment/platform-api -n api-anything-production
```

---

## 7. 常用命令速查表

### 开发

| 操作 | 命令 |
|------|------|
| 启动基础设施 | `cd docker && podman-compose up -d` |
| 停止基础设施 | `cd docker && podman-compose down` |
| 停止并清除数据 | `cd docker && podman-compose down -v` |
| 运行数据库迁移 | `psql "$DATABASE_URL" -f crates/metadata/src/migrations/20260321000000_initial_schema.sql` |
| 编译全部 | `cargo build --workspace` |
| 启动后端 | `cargo run -p api-anything-platform-api` |
| 启动前端 | `cd web && npm run dev` |
| 前端构建 | `cd web && npm run build` |
| 访问 Grafana | `open http://localhost:3000` |

### 测试

| 操作 | 命令 |
|------|------|
| 运行全部测试 | `cargo test --workspace` |
| 运行指定 crate 测试 | `cargo test -p api-anything-<crate>` |
| 运行指定测试函数 | `cargo test --workspace <test_name>` |
| 测试覆盖率（终端） | `cargo llvm-cov --workspace` |
| 测试覆盖率（HTML） | `cargo llvm-cov --workspace --html --output-dir target/coverage` |
| 前端 Lint | `cd web && npm run lint` |

### 代码质量

| 操作 | 命令 |
|------|------|
| 格式化 | `cargo fmt --all` |
| 格式检查 | `cargo fmt --all -- --check` |
| Clippy 检查 | `cargo clippy --workspace --all-targets -- -D warnings` |

### 容器镜像

| 操作 | 命令 |
|------|------|
| 构建前端镜像 | `cd web && podman build -t api-anything-web:latest .` |
| 构建后端镜像 | `podman build -t api-anything-platform-api:latest -f Dockerfile.platform-api .` |
| 推送镜像 | `podman push ghcr.io/<org>/api-anything-platform-api:<tag>` |

### 健康检查

| 操作 | 命令 |
|------|------|
| Liveness | `curl http://localhost:8080/health` |
| Readiness | `curl http://localhost:8080/health/ready` |
| 查看 API 文档 | `curl http://localhost:8080/api/v1/docs` |
| PostgreSQL 就绪检查 | `pg_isready -h localhost -U api_anything -d api_anything` |

### Kubernetes

| 操作 | 命令 |
|------|------|
| 查看 Pod 状态 | `kubectl get pods -n api-anything-production` |
| 查看日志 | `kubectl logs -f deployment/platform-api -n api-anything-production` |
| 重启部署 | `kubectl rollout restart deployment/platform-api -n api-anything-production` |
| 回滚部署 | `kubectl rollout undo deployment/platform-api -n api-anything-production` |
| 查看 HPA 状态 | `kubectl get hpa -n api-anything-production` |
| 手动扩缩容 | `kubectl scale deployment/platform-api --replicas=5 -n api-anything-production` |
