# API-Anything 平台全局架构设计规格书

## 1. 项目概述

### 1.1 愿景

API-Anything 是一个基于 LLM 和 Rust 生态构建的全自动企业级 API 网关生成平台。它将遗留系统（SOAP、OData、CLI、SSH、PTY）通过 AI 驱动的 7 阶段流水线，自动转换为高性能、强类型、自带全链路监控的 REST API 服务。

### 1.2 核心决策摘要

| 决策项 | 选择 | 理由 |
|--------|------|------|
| 项目推进策略 | 全局设计 + 分期实施 | 子系统深度耦合，先定义接口边界避免返工 |
| 优先协议支持 | SOAP > CLI > SSH > PTY | 按业务优先级排序，闭源二进制暂不纳入 |
| 并发策略 | 集群水平扩展 + 后端保护 | 真正瓶颈在遗留后端，非网关自身吞吐 |
| 沙箱模式 | Mock + Replay + Proxy 三层 | 不同阶段不同需求，分层覆盖 |
| 投递保障 | 默认最终一致性 + 可选幂等 | 务实选择，避免过度设计 |
| 生成模式 | Rust 编译插件 + 声明式热加载配置 | 重逻辑编译保类型安全，轻配置热更新保灵活 |
| LLM 集成 | 纯离线生成，运行时零 AI 依赖 | 千万级用户规模下在线 LLM 不可控 |
| 交付形态 | CLI 核心 + Web 管理辅助 | CLI 自动化友好，Web 适合管理运营 |
| 基础设施 | K8s + Kafka + PostgreSQL + OTel | 通用云原生，组件可插拔 |
| 架构模式 | 元数据驱动平台 | 统一事实源，一次生成全平台受益 |

## 2. 系统架构

### 2.1 架构总览

```
┌─────────────────────────────────────────────────────┐
│              接入层 (CLI + Web + CI/CD)               │
└────────────────────┬────────────────────────────────┘
                     │
┌────────────────────▼────────────────────────────────┐
│              Platform API (Rust/Axum)                │
│         统一后端，CLI 和 Web 共享                     │
└────────────────────┬────────────────────────────────┘
                     │
┌════════════════════▼════════════════════════════════┐
║          元数据仓库 (PostgreSQL)                      ║
║  ┌─────────┬──────────┬──────────┬───────────┐      ║
║  │契约模型  │路由拓扑   │生成产物   │运行状态    │      ║
║  └─────────┴──────────┴──────────┴───────────┘      ║
╚═════╤═══════════╤═══════════╤═══════════╤═══════════╝
      │           │           │           │
┌─────▼───┐ ┌────▼────┐ ┌───▼────┐ ┌────▼──────┐
│ 生成引擎 │ │网关运行时│ │沙箱引擎 │ │ 补偿引擎   │
│(离线任务)│ │(在线服务)│ │(按需启停)│ │(消费者)    │
└─────────┘ └────┬────┘ └────────┘ └───────────┘
                 │
           ┌─────▼─────┐
           │ 遗留系统    │
           │ SOAP/CLI/  │
           │ SSH/PTY    │
           └───────────┘

  ┌─────────────────────────────────────────────────┐
  │     横切关注点 (所有组件共享)                      │
  │  OTel Tracing │ Kafka 事件总线 │ 配置热加载       │
  └─────────────────────────────────────────────────┘
```

### 2.2 核心设计原则

- **元数据即唯一事实源**：所有子系统从 PostgreSQL 元数据仓库读取契约、路由、配置，LLM 生成产物写入后全平台自动感知
- **无状态网关**：服务不维护业务 Session，Pod 可任意横向扩展
- **后端保护优先**：架构精力聚焦在熔断、限流、信号量隔离，保护脆弱的遗留系统
- **离线生成、在线执行**：LLM 仅在生成阶段参与，运行时零 AI 依赖
- **渐进式演进**：初期可合并部署为 2-3 个进程，后期按需拆分

## 3. 元数据模型

### 3.1 核心实体

#### Project（项目）
```
Project
├── id: UUID
├── name: String
├── description: String
├── owner: String
├── source_type: Enum(WSDL, OData, CLI, SSH, PTY)
├── source_config: EncryptedJSON  // 源系统连接信息，加密存储
├── created_at, updated_at: Timestamp
```

#### Contract（契约 — LLM 解析产物）
```
Contract
├── id: UUID
├── project_id: UUID → Project
├── version: SemVer
├── status: Enum(draft, active, deprecated)
├── original_schema: Text          // 原始契约文本 (WSDL/man页等)
├── parsed_model: JSONB            // 统一中间表示 (JSON Schema)
├── created_at, updated_at: Timestamp
```

#### Route（路由拓扑）
```
Route
├── id: UUID
├── contract_id: UUID → Contract
├── method: Enum(GET, POST, PUT, DELETE)
├── path: String                   // e.g., /api/v1/orders/{id}
├── request_schema: JSONB          // JSON Schema
├── response_schema: JSONB         // JSON Schema
├── transform_rules: JSONB         // 字段映射/类型转换规则（热加载）
├── backend_binding_id: UUID → BackendBinding
├── delivery_guarantee: Enum(at_most_once, at_least_once, exactly_once)
├── enabled: Boolean
├── created_at, updated_at: Timestamp
```

#### BackendBinding（后端绑定 — 核心抽象）
```
BackendBinding
├── id: UUID
├── route_id: UUID → Route
├── protocol: Enum(SOAP, HTTP, CLI, SSH, PTY)
├── endpoint_config: EncryptedJSON // 连接地址/凭证等
├── connection_pool_config: JSONB  // 连接池/信号量配置
│   ├── max_connections: u32
│   ├── idle_timeout: Duration
│   └── max_lifetime: Duration
├── circuit_breaker_config: JSONB  // 熔断策略
│   ├── error_threshold_percent: f32
│   ├── window_duration: Duration
│   ├── open_duration: Duration
│   └── half_open_max_requests: u32
├── rate_limit_config: JSONB       // 限流策略
│   ├── requests_per_second: u32
│   └── burst_size: u32
├── retry_config: JSONB            // 重试策略
│   ├── max_retries: u32
│   ├── base_delay: Duration
│   └── max_delay: Duration
├── timeout: Duration
├── auth_mapping: JSONB            // 鉴权翻译规则
```

#### Artifact（生成产物）
```
Artifact
├── id: UUID
├── contract_id: UUID → Contract
├── artifact_type: Enum(plugin_so, config_yaml, openapi_json, dockerfile, test_suite, agent_prompt)
├── content_hash: String (SHA-256)
├── storage_path: String
├── build_status: Enum(building, ready, failed)
├── build_log: Text
├── created_at: Timestamp
```

#### DeliveryRecord（投递记录 — 补偿引擎）
```
DeliveryRecord
├── id: UUID
├── route_id: UUID → Route
├── trace_id: String
├── idempotency_key: String (nullable, exactly_once 模式必填)
├── request_payload: JSONB
├── response_payload: JSONB (nullable)
├── status: Enum(pending, delivered, failed, dead)
├── retry_count: u32
├── next_retry_at: Timestamp (nullable)
├── error_message: Text (nullable)
├── created_at, updated_at: Timestamp
```

#### SandboxSession（沙箱会话）
```
SandboxSession
├── id: UUID
├── project_id: UUID → Project
├── tenant_id: String             // 租户隔离标识
├── mode: Enum(mock, replay, proxy)
├── config: JSONB                  // 模式特定配置
├── expires_at: Timestamp
├── created_at: Timestamp
```

#### RecordedInteraction（录制的交互）
```
RecordedInteraction
├── id: UUID
├── session_id: UUID → SandboxSession
├── route_id: UUID → Route
├── request: JSONB
├── response: JSONB
├── duration_ms: u32
├── recorded_at: Timestamp
```

### 3.2 关键设计决策

- **BackendBinding 是核心抽象**：统一了所有后端交互模式，网关运行时只面向 BackendBinding 接口编程
- **Contract 版本控制**：同一遗留系统可有多个版本的契约，支持灰度发布和回滚
- **transform_rules 与 plugin_so 分离**：轻量字段映射走热加载配置，重逻辑走编译插件
- **DeliveryRecord 支撑补偿引擎**：每次需保障投递的请求都记录，失败后按 retry_config 自动重推，超限进死信队列

## 4. AI 生成引擎（7 阶段流水线）

### 4.1 流水线总览

```
输入源 → Stage 1~7 → 输出产物

输入:                          输出:
  WSDL/XML                       Rust Plugin (.so)
  OData URL                      Transform Config (YAML)
  man 手册                       OpenAPI 3.0 Spec
  SSH 信息                       Route Definitions
  CLI 帮助                       Shadow Test Suite
  终端输出                       Dockerfile
                                 Agent Prompt
```

### 4.2 各阶段详细设计

#### Stage 1: 输入解析（Contract Parser）

| 输入类型 | 解析策略 | LLM 参与度 |
|---------|---------|-----------|
| SOAP/WSDL | `quick-xml` 结构化解析，提取 operation/type，LLM 补充语义描述 | 中 |
| OData XML | 解析 $metadata，提取 EntityType/EntitySet/Action | 低 |
| CLI/Shell | 解析 `--help`/`man` 输出，提取子命令/参数/选项 | 高 |
| SSH/Telnet | 用户提供交互样例文本，LLM 理解提示符/命令/输出模式 | 高 |

**RAG 分块处理**：大型 WSDL（数万行）按 `<wsdl:portType>` 切分为独立块，每块独立送入 LLM，最后合并输出并检查跨块引用一致性。

#### Stage 2: 统一建模（Schema Unifier）

所有输入统一映射为中间表示：

```rust
struct UnifiedContract {
    operations: Vec<Operation>,       // 操作列表
    types: Vec<TypeDef>,              // 类型定义 (JSON Schema)
    routes: Vec<RouteMapping>,        // REST 路由映射
    auth_requirements: AuthSpec,      // 鉴权需求
    error_patterns: Vec<ErrorPattern>, // 错误模式识别
}
```

LLM 决定 REST 化的最佳映射：
- SOAP `getOrderById` → `GET /api/v1/orders/{id}`
- CLI `ls --format=json /var/log` → `GET /api/v1/logs?format=json`
- SSH `show interface status` → `GET /api/v1/interfaces/status`

#### Stage 3: 代码生成（Code Generator）

根据协议类型生成对应的 Rust 适配器：

| 协议 | 技术栈 |
|------|--------|
| SOAP | reqwest + quick-xml 序列化/反序列化 |
| HTTP | reqwest 直接转发 + serde 映射 |
| CLI | tokio::process::Command + .arg() 安全传参 |
| SSH | ssh2-rs 连接 + 命令执行 + 输出解析 |
| PTY | rexpect 伪终端 + Expect 状态机 |

**安全硬性规则（编译到生成模板中）**：
- CLI 参数禁止字符串拼接，强制 `.arg()` 列表传参
- SSH 凭证从 Vault/K8s Secret 读取，禁止硬编码
- 所有生成代码自动包含 `#[tracing::instrument]`

#### Stage 4: 测试生成

- 影子测试（Shadow Test）：对网络转换数据或 CLI 文本提取正则进行 `assert_json_diff` 深度断言比对
- 边界用例：空值、超长字段、特殊字符、编码问题

#### Stage 5: 文档生成

- 通过 `utoipa` 宏自动生成 OpenAPI 3.0 规范
- 同时生成 Agent 提示词（结构化 Prompt）

#### Stage 6: 观测注入

- 自动为所有函数打上 `#[tracing::instrument]` 宏
- 注入 span attributes（protocol、backend、command 等）

#### Stage 7: 构建打包

- 编译 Rust Plugin 为 `.so` 动态库
- 生成 Dockerfile（musl-libc 静态编译，FROM scratch，< 20MB）

### 4.3 增量生成（Refine Mode）

- 检测元数据变更，通过 AST diff 确定影响范围，仅重新生成受影响的代码块
- 配置类变更（IP、超时）直接更新 YAML，无需触发生成流水线

## 5. 网关运行时（Gateway Runtime）

### 5.1 请求处理管道

```
请求流入 → Axum HTTP Server

中间件管道（按序执行）:
  ① TLS 终结 (rustls)
  ② TraceLayer (W3C traceparent 生成/透传)
  ③ Auth Guard (JWT 验证 → 凭证翻译)
  ④ Rate Limiter (令牌桶, 按路由/租户独立配置)
  ⑤ Request Logger (请求快照, 补偿引擎用)

→ Dynamic Router (动态路由器)
  从元数据加载路由表, 匹配 Route → Plugin + Config

→ Backend Dispatcher (后端调度器)
  Protocol Adapter (Plugin 实现) + 保护层

→ Response Pipeline (响应管道)
  Transform → Error Normalizer → Delivery Hook
```

### 5.2 动态路由热加载

- 启动时从 PostgreSQL 加载全量路由表到内存
- 运行时感知变更的两种方式：
  - **主动轮询**：每 5s 检查路由版本号（轻量 SQL 查询）
  - **Kafka 事件**：生成引擎完成后发送 `route.updated` 事件，网关即时响应
- 路由更新采用 **RCU（Read-Copy-Update）** 策略：构建新路由表 → 原子指针替换 → 旧表延迟释放，请求处理零中断

### 5.3 Plugin 动态加载

通过 `libloading` 加载生成的 `.so` 插件，每个 Plugin 实现统一 trait：

```rust
trait ProtocolAdapter: Send + Sync {
    async fn transform_request(&self, req: GatewayRequest) -> BackendRequest;
    async fn execute(&self, req: BackendRequest) -> BackendResponse;
    async fn transform_response(&self, resp: BackendResponse) -> GatewayResponse;
}
```

Plugin 更新流程：加载新版本 → 健康检查通过 → 原子替换 → 卸载旧版本。

### 5.4 后端保护策略

按协议类型差异化配置默认值（可通过元数据按路由独立覆盖）：

| 后端类型 | 默认并发上限 | 熔断阈值 | 超时 | 原因 |
|---------|------------|---------|------|------|
| SOAP/HTTP | 1000 | 50% 错误率/30s | 30s | 网络服务本身有一定承载力 |
| CLI (本地) | 10 | 30% 错误率/10s | 60s | 进程 Fork 消耗 OS 资源 |
| SSH (远程) | 5 | 20% 错误率/10s | 120s | 远程服务器通常更脆弱 |
| PTY (会话) | 3 | 20% 错误率/10s | 300s | 交互式会话最重，需严格控制 |

保护层组件：
- **Circuit Breaker（熔断器）**：滑动窗口错误率统计，三态流转 Closed → Open → Half-Open
- **Semaphore（并发信号量）**：每个 BackendBinding 独立配置上限
- **Connection Pool（连接池）**：HTTP/SOAP 用 deadpool + reqwest 连接复用，SSH 用 ssh2-rs 会话池，PTY 用伪终端会话池（带健康检查）

### 5.5 鉴权映射层

网关对外统一 JWT/OAuth2，内部根据 `BackendBinding.auth_mapping` 翻译凭证：

| 后端协议 | 凭证翻译 |
|---------|---------|
| SOAP | 注入 WS-Security Header |
| CLI | 映射到 Linux 用户（受限 sudo） |
| SSH | 从 Vault 获取对应权限的 SSH Key |
| PTY | 注入用户名密码到 Expect 序列 |

### 5.6 错误规范化引擎

所有后端返回统一走 Error Normalizer，输出标准 RFC 7807 (Problem Details for HTTP APIs)：

| 错误来源 | 规范化策略 |
|---------|-----------|
| CLI 退出码 ≠ 0 | 解析 stderr → RFC 7807 |
| CLI 退出码 = 0 但含错误文本 | LLM 预生成的 regex 匹配 → RFC 7807 |
| SOAP Fault | 提取 faultcode/faultstring → RFC 7807 |
| SSH 连接失败/超时 | 标准化为 502/504 |

## 6. 沙箱测试平台（Sandbox Engine）

### 6.1 三层递进模式

沙箱通过独立端口提供服务，URL 结构与生产完全一致，下游切换环境只需改 base URL。

#### Layer 1: Mock 模式

- **触发方式**：请求头 `X-Sandbox-Mode: mock`
- **数据来源**：从元数据 `Route.request_schema` / `response_schema` 自动生成
- **生成策略**：
  - **Smart Mock**：根据字段名语义生成合理数据（email → `user@example.com`，amount → `99.50`）
  - **Schema Mock**：严格按 JSON Schema 类型生成随机值
  - **Fixed Mock**：用户自定义固定响应
- **特点**：零依赖真实后端，毫秒级响应
- **适用阶段**：下游早期开发，快速验证接口格式

#### Layer 2: Replay 模式

- **触发方式**：请求头 `X-Sandbox-Mode: replay`
- **数据来源**：预录制的真实请求-响应对
- **匹配策略**：
  - **精确匹配**：URL + Body 完全一致
  - **模糊匹配**：忽略时间戳/随机字段，按业务键匹配
  - **无匹配时**：返回 404 + 最相似的已录制请求提示
- **录制来源**：Proxy 模式自动录制 或 手动导入
- **适用阶段**：联调/回归测试，需要真实数据结构

#### Layer 3: Proxy 模式

- **触发方式**：请求头 `X-Sandbox-Mode: proxy`
- **行为**：请求透传到真实后端的测试环境，同时自动录制交互
- **隔离机制**：
  - **租户标记**：`X-Sandbox-Tenant` 注入到后端请求头，测试环境据此隔离数据
  - **数据染色**：请求/响应自动添加 `_sandbox: true` 标记，防止污染生产
  - **读写控制**：可配置为只读模式（仅允许 GET），防止测试写入破坏测试环境
- **适用阶段**：预发布验证，需要端到端真实验证

### 6.2 关键机制

- **自动可用**：每个生成的 API 自动附带 Mock 沙箱，无需额外配置
- **录制引擎**：Proxy 模式自动录制所有交互，一键转为 Replay 数据集
- **交互面板**：Web 面板实时展示请求日志、匹配率、延迟分布，提供 cURL 示例和 SDK 代码片段
- **自助服务**：下游通过 Web 面板自行创建沙箱会话、查看录制数据、下载测试报告

## 7. 数据补偿与重推引擎（Compensation Engine）

### 7.1 三级投递保障

| 级别 | 配置值 | 行为 | 适用场景 |
|------|--------|------|---------|
| 尽力投递 | `at_most_once` | 失败不重试，不记录 | 日志查询、监控数据等幂等且非关键的读操作 |
| 至少一次 | `at_least_once` | 自动重试 + 死信队列，下游需自行处理重复 | 通知、状态同步等可容忍重复的场景 |
| 精确一次 | `exactly_once` | 幂等键保障 + 自动重试 + 死信队列 | 订单创建、资金操作等业务关键场景 |

每个路由通过元数据 `Route.delivery_guarantee` 独立配置。

### 7.2 核心组件

#### Request Logger 中间件

位于网关请求管道中，判断路由的投递保障级别：
- `at_most_once`：不记录，失败即失败
- `at_least_once`：记录请求快照 → Kafka `delivery-events` topic
- `exactly_once`：记录 + 幂等键校验 → Kafka

中间件异步写入 Kafka，不阻塞网关热路径。

#### Compensation Worker（消费者集群）

独立进程，从 Kafka 消费失败的投递事件：

**指数退避重试策略**：
```
1st retry:  1s
2nd retry:  5s
3rd retry:  30s
4th retry:  5min
5th retry:  30min
超出 max_retries → Dead Letter Queue
```

每次重试前：
1. 检查熔断器状态（Open 则延迟到 Half-Open）
2. 检查幂等键（`exactly_once` 模式防重复投递）
3. 重新执行后端调用（复用 Plugin 逻辑，加载相同 `.so`）

#### Idempotency Guard（幂等保障）

PostgreSQL 表 `idempotency_keys`：

| 字段 | 类型 | 说明 |
|------|------|------|
| idempotency_key (PK) | String | 客户端提供的幂等键 |
| route_id | UUID | 关联路由 |
| status | Enum | pending / delivered |
| response_hash | String | 成功响应的哈希（用于缓存返回） |

流程：
- 收到请求 → 查幂等键
  - 已存在 + delivered → 直接返回缓存响应
  - 已存在 + pending → 等待/拒绝（防并发重复）
  - 不存在 → 插入 pending → 执行 → 更新状态

#### Dead Letter Processor（死信处理）

超过最大重试次数的消息进入死信队列：
1. 持久化到 PostgreSQL `delivery_records` 表
2. 触发告警 → OTel Metric + Webhook 通知
3. Web 面板展示死信列表，支持：
   - 人工审查失败原因
   - 一键重推（单条/批量）
   - 修改 payload 后重推
   - 标记为已处理（人工确认不需要重推）

### 7.3 主动推送（Push Dispatcher）

支持网关主动向下游推送数据（Webhook/回调）：

```
上游事件 → Kafka push-events → Push Worker → 下游 endpoint
                                    │
                                    └→ 失败 → 进入 Retry Scheduler
```

下游订阅管理：
- Web 面板注册 Webhook URL
- 配置订阅的事件类型和过滤条件
- 配置推送格式（JSON / XML / 自定义模板）
- 查看推送历史和成功率

### 7.4 与网关运行时的关系

- 补偿引擎**不在网关热路径上** — 网关只负责将请求快照异步写入 Kafka
- Compensation Worker 是独立进程，可独立扩缩
- 重试时复用生成的 Plugin 逻辑，保证行为一致性

## 8. 文档与开发者门户（Developer Portal）

### 8.1 自动生成层（零人工）

| 组件 | 数据来源 | 更新机制 |
|------|---------|---------|
| OpenAPI 3.0 Spec | 元数据 Route + Schema | 路由变更后实时更新（监听 Kafka 事件） |
| API 变更日志 | Contract 版本 diff | 自动生成变更摘要，标注 Breaking Change |
| Agent 提示词 | Route + Schema + 描述 | 每个 API 自动生成结构化 Prompt |

核心原则：**文档即代码（Docs as Code）**，所有文档从元数据自动派生，下游看到的文档永远与实际 API 一致。

### 8.2 开发者工具层

| 组件 | 功能 | 技术实现 |
|------|------|---------|
| Swagger UI | OpenAPI 交互式文档 | 嵌入标准 Swagger UI |
| API Explorer | 类 Postman 在线调试 | 自研组件，基于 Schema 自动构建表单，支持切换生产/沙箱 |
| SDK 代码生成 | 客户端代码生成 | OpenAPI Generator，支持 TypeScript/Python/Java/Go |
| 沙箱入口 | 沙箱会话管理 | 创建会话、查看录制、下载测试报告 |

### 8.3 运营管理层

| 组件 | 功能 |
|------|------|
| 项目管理 | 遗留系统接入项目列表、生成流水线状态、Contract 版本管理 |
| 监控面板 | Grafana 嵌入（OTel 指标），路由级 QPS/延迟/错误率，熔断器/连接池状态 |
| 补偿管理 | 死信队列浏览、重推操作、投递成功率趋势 |

### 8.4 技术选型

| 组件 | 技术 | 说明 |
|------|------|------|
| Web 前端 | React + TypeScript | SPA，调用 Platform API |
| 监控面板 | Grafana iframe 嵌入 | 复用 OTel 数据，不重复造轮子 |
| 实时更新 | WebSocket | 监听 Kafka 事件，推送文档/状态变更 |
| SDK 生成 | OpenAPI Generator | 服务端按需生成，缓存结果 |

## 9. 全链路监控与可观测性（Observability）

### 9.1 数据采集

所有组件统一使用 OpenTelemetry SDK，通过 OTLP (gRPC) 发送到 OTel Collector。

OTel Collector Processors：
- **Batch**：批量发送降低网络开销
- **Tail Sampling**：尾采样策略控制数据量
- **Attribute**：注入环境标签（cluster、namespace、pod）

### 9.2 网关关键埋点

一个典型请求的 Trace 瀑布流：

```
[Trace: abc-123]
├─ gateway.request                           0ms ────── 185ms
│  ├─ middleware.auth                         1ms ── 3ms
│  ├─ middleware.rate_limit                   3ms ── 4ms
│  ├─ router.dispatch                        4ms ── 5ms
│  ├─ plugin.transform_request               5ms ── 8ms
│  ├─ backend.execute                        8ms ──── 178ms
│  │  ├─ backend.circuit_breaker.check        8ms ── 8ms
│  │  └─ [protocol].request/execute          9ms ── 175ms
│  ├─ plugin.transform_response            178ms ── 182ms
│  ├─ response.error_normalize             182ms ── 183ms
│  └─ delivery.record                      183ms ── 185ms
```

### 9.3 核心指标

| 指标名 | 类型 | 维度 | 告警阈值 |
|--------|------|------|---------|
| `gateway.request.duration` | Histogram | route, method, status | P99 > 5s |
| `gateway.request.total` | Counter | route, method, status | — |
| `backend.execute.duration` | Histogram | route, protocol, backend | P99 > 10s |
| `backend.circuit_breaker.state` | Gauge | route, backend | state = open |
| `backend.connection_pool.usage` | Gauge | route, backend | > 80% |
| `backend.semaphore.queued` | Gauge | route, backend | > 50 |
| `delivery.retry.total` | Counter | route, status | — |
| `delivery.dead_letter.total` | Counter | route | 任何增长 |
| `generation.pipeline.duration` | Histogram | project, stage | — |
| `sandbox.request.total` | Counter | project, mode | — |

### 9.4 告警策略

| 级别 | 触发条件 | 响应时间 | 通知渠道 |
|------|---------|---------|---------|
| P0 | 熔断器打开 / 死信队列增长 / 5xx > 5% | 立即 | PagerDuty/电话 + Slack/钉钉 |
| P1 | P99 超阈值 / 连接池 > 80% / 信号量排队 > 50 | 30 分钟 | Slack/钉钉 |
| P2 | 生成流水线失败 / 沙箱匹配率下降 | 下个工作日 | 邮件 |

### 9.5 分层存储策略

按千万级用户规模（~50K QPS），追踪数据量巨大，采用分层存储 + 生命周期管理：

| 层级 | 保留时间 | 存储介质 | 预估容量 | 查询延迟 |
|------|---------|---------|---------|---------|
| Hot（热数据） | 24h | 内存/SSD | ~1TB | < 1s |
| Warm（温数据） | 7~30 天 | 对象存储 (S3/OSS/MinIO) | ~15TB | 1~10s |
| Cold（冷数据） | 30~90 天 | 对象存储低频层 | ~30TB (Parquet 列存压缩) | 10s~分钟级 |
| Frozen（归档） | 超出保留期 | 只保留聚合指标，原始数据删除 | ~2TB | — |

**存储成本优化关键手段：**

1. **采样策略（源头减量）**：正常请求 10% 采样，错误/慢请求/补偿重试 100% 全量采集。Trace 总量减少 ~80-90%
2. **压缩与编码**：Tempo zstd 压缩，Loki snappy 压缩，Parquet 列存压缩比 ~10:1。存储空间再减少 60~80%
3. **智能降采样（Metrics）**：Hot 15s 间隔 → Warm 1min → Cold 5min。长期存储量减少 95%
4. **Trace 裁剪**：正常 Trace 只保留入口 span + 后端调用 span，中间件 span 仅记录耗时异常的

**技术选型：**

| 数据类型 | 方案 | 原因 |
|---------|------|------|
| Traces | Grafana Tempo | 原生 S3 后端，无需 Elasticsearch，成本极低 |
| Metrics | Prometheus + Thanos | 本地短期存储 + Thanos Sidecar 自动上传 S3 长期存储 |
| Logs | Grafana Loki | 只索引标签不索引全文，成本比 ES 低一个量级 |

三者统一用 Grafana 做查询和可视化。

## 10. 分期实施路线图

### Phase 0: 基础设施（第 1~2 周）

- 项目脚手架：Rust workspace (Cargo.toml)
- 元数据仓库：PostgreSQL schema + migration (sqlx)
- Platform API 骨架：Axum + 基础中间件
- Kafka 集群 + Topic 定义
- OTel Collector + Tempo + Prometheus + Loki + Grafana
- Docker Compose 本地开发环境
- CI/CD Pipeline 骨架

### Phase 1: 核心引擎 — SOAP/WSDL → REST（第 3~6 周）

- 输入解析器：WSDL Parser (quick-xml + LLM)
- 统一建模：Contract → JSON Schema → Route
- 代码生成：Rust SOAP 适配器 Plugin (.so)
- 网关运行时：动态路由 + Plugin 加载 + SOAP 后端调度
- 保护层：限流 + 熔断 + 连接池（SOAP/HTTP 场景）
- 错误规范化：SOAP Fault → RFC 7807
- 测试生成：影子测试 + 深度断言
- 文档生成：OpenAPI 3.0 + Swagger UI
- **验收标准**：能将一个真实 WSDL 自动转为可用的 REST API

### Phase 2: CLI/SSH 扩展（第 7~10 周）

- 输入解析器：CLI help/man 解析 + SSH 交互样例解析
- 代码生成：CLI 适配器 (tokio::process) + SSH 适配器 (ssh2-rs)
- 保护层：进程信号量 + SSH 会话池 + 差异化限流
- 错误规范化：Exit Code + stderr → RFC 7807
- LLM 文本解析：regex/nom 提取逻辑生成
- **验收标准**：能将 CLI 工具和 SSH 命令自动包装为 REST API

### Phase 3: 沙箱测试平台（第 11~14 周）

- Mock Layer：Schema → 智能假数据生成
- Replay Layer：录制引擎 + 匹配回放
- Proxy Layer：流量透传 + 租户隔离 + 数据染色
- 沙箱 Gateway：独立端口 + 模式路由
- 沙箱管理 API：会话 CRUD + 录制数据管理
- **验收标准**：下游系统可通过沙箱独立完成联调

### Phase 4: 数据补偿引擎（第 15~18 周）

- Request Logger 中间件：请求快照 → Kafka
- Compensation Worker：消费 + 指数退避重试
- Idempotency Guard：幂等键校验 (PostgreSQL)
- Dead Letter Processor：死信队列 + 告警
- Push Dispatcher：主动推送 + 订阅管理
- 管理 API：死信查看 + 手动重推 + 批量操作
- **验收标准**：生产环境具备失败自动恢复能力

### Phase 5: 开发者门户 + PTY 扩展（第 19~24 周）

- Web 前端：React + TypeScript
- API Explorer：交互式调试器
- SDK 生成：OpenAPI Generator 集成
- 变更日志：Contract diff + Breaking Change 标注
- 监控面板：Grafana 嵌入 + 告警配置
- 补偿管理界面：死信处理 + 重推操作
- PTY 适配器：rexpect + Expect 状态机
- 整体集成测试 + 性能压测 + 文档完善
- **验收标准**：平台具备完整的自助服务能力

## 11. 全链路自动化测试策略

贯穿所有实施阶段：

| 测试类型 | 覆盖范围 | 执行时机 |
|---------|---------|---------|
| 单元测试 | 各模块核心逻辑（解析器、映射器、序列化） | 每次 commit |
| 集成测试 | 模块间交互（生成引擎→元数据→网关加载） | 每次 PR |
| 契约测试 | 生成的 API 与 OpenAPI Spec 一致性 | 每次生成 |
| 影子测试 | 生成代码 vs 真实后端响应的 Deep Equal | 每次生成 |
| 端到端测试 | 完整链路：输入契约→生成→部署→调用→响应 | 每日 CI |
| 性能测试 | 网关吞吐、延迟、后端保护策略验证 | 每周/Release |
| 混沌测试 | 后端宕机、网络分区、Kafka 不可用 | 每月 |

## 12. 技术栈总览

| 层面 | 技术 | 用途 |
|------|------|------|
| 核心语言 | Rust | 网关运行时、生成引擎、CLI 工具 |
| Web 框架 | Axum | HTTP 服务、中间件管道 |
| 异步运行时 | Tokio | 异步调度、进程管理 |
| 序列化 | serde + quick-xml | JSON/XML 双向转换 |
| HTTP 客户端 | reqwest | SOAP/HTTP 后端请求 |
| SSH | ssh2-rs | 远程终端连接 |
| PTY | rexpect | 伪终端交互 |
| 动态库 | libloading | Plugin .so 加载 |
| 数据库 | PostgreSQL + sqlx | 元数据仓库 |
| 消息队列 | Kafka | 事件总线、补偿引擎 |
| 连接池 | deadpool | 后端连接复用 |
| TLS | rustls | HTTPS 加密 |
| 加解密 | ring | 数据链路安全 |
| 追踪 | tracing + OpenTelemetry | 分布式追踪 |
| 前端 | React + TypeScript | 开发者门户 |
| 文档 | utoipa + Swagger UI | OpenAPI 3.0 自动生成 |
| 容器 | Docker (musl-libc, FROM scratch) | < 20MB 极轻量镜像 |
| 编排 | Kubernetes + HPA | 弹性伸缩 |
| 监控 | Grafana + Tempo + Prometheus + Loki + Thanos | 统一可观测性 |
| LLM | 多模型适配层（Claude/OpenAI/DeepSeek） | 离线契约解析和代码生成 |
