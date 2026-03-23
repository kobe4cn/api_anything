# API-Anything

**AI 驱动的遗留系统 API 网关生成平台** — 将 SOAP、CLI、SSH、PTY 等遗留协议一键转换为现代 REST API。

![Rust](https://img.shields.io/badge/Rust-1.82+-orange?logo=rust)
![License](https://img.shields.io/badge/License-MIT-blue)
![Tests](https://img.shields.io/badge/Tests-342%20passed-brightgreen)
![Crates](https://img.shields.io/badge/Crates-10-purple)
![LLM Providers](https://img.shields.io/badge/LLM%20Providers-7-green)

> **10** Rust Crates | **8** Web 页面 | **342** 自动化测试 | **6** 协议类型 | **7** LLM 厂商 | **16** 前端截图

---

## 目录

- [项目概述](#项目概述)
- [亮点功能](#亮点功能)
- [系统架构](#系统架构)
- [快速开始](#快速开始)
- [项目结构](#项目结构)
- [核心模块详解](#核心模块详解)
- [Web 管理平台](#web-管理平台)
- [API 端点速查](#api-端点速查)
- [CLI 工具](#cli-工具)
- [配置参考](#配置参考)
- [测试](#测试)
- [技术栈](#技术栈)
- [文档索引](#文档索引)
- [路线图](#路线图)
- [License](#license)

---

## 项目概述

API-Anything 是一个基于 **LLM 和 Rust 生态** 构建的全自动企业级 API 网关生成平台。它面向拥有大量遗留系统的企业，解决一个普遍而棘手的问题：**如何以最低成本将 SOAP Web Service、命令行工具、SSH 终端设备、PTY 交互程序等非 REST 系统，安全、高效地现代化为标准的 REST API**。

传统方案要么需要人工逐个编写适配层（高成本、低效率），要么依赖运行时 AI 推理（不可控、高延迟）。API-Anything 走了第三条路：**离线阶段由 LLM 驱动代码生成引擎，直接产出可编译的 Rust 插件（.so/.dylib），运行时完全确定性执行，零 AI 依赖**。这种「AI 驱动设计期，确定性保障运行期」的架构，兼顾了智能化和可靠性。

核心理念：

- **LLM 驱动代码生成** — 7 阶段流水线：LLM 分析语义 → 生成强类型 Rust 代码 → 编译为动态库 → 自动热加载
- **多厂商 LLM 支持** — 7 家 LLM 厂商（Anthropic/OpenAI/Gemini/GLM/Qwen/Kimi/DeepSeek）统一适配
- **6 种协议适配** — SOAP/OData/OpenAPI/CLI/SSH/PTY 全覆盖，一键转为 REST API
- **零运行时 LLM 依赖** — 生成的插件是原生 Rust 编译产物，运行时完全确定性执行，支撑千万级请求规模

---

## 亮点功能

| | 功能 | 描述 |
|---|---|---|
| 🔄 | **六协议适配** | SOAP/OData/OpenAPI/CLI/SSH/PTY 统一通过 `ProtocolAdapter` trait 适配，一键转为 REST API |
| 🧠 | **LLM 代码生成引擎** | 7 阶段流水线：LLM 分析语义 → 生成 Rust 源码 → 编译 .so/.dylib → 失败自动修复 → 热加载上线 |
| 🤖 | **7 家 LLM 厂商** | Anthropic/OpenAI/Gemini/GLM/Qwen/Kimi/DeepSeek 统一适配，自由切换 |
| 🛡️ | **三层后端保护** | 令牌桶限流 + 滑动窗口熔断（三态） + 并发信号量，协议感知默认值 |
| 🧪 | **三模式沙箱** | Mock（Schema 驱动）/ Replay（录制回放）/ Proxy（真实代理），分层联调 |
| 🔁 | **数据补偿引擎** | 自动指数退避重试 + 幂等键精确一次 + 死信管理 API |
| 📡 | **事件总线** | PG 轮询默认实现 + Kafka 可选扩展，7 种业务事件类型覆盖核心生命周期 |
| 🔌 | **插件系统** | C ABI `.so` 动态加载，`export_plugin!` 宏简化开发，用户自定义协议扩展 |
| 📊 | **全链路可观测** | OpenTelemetry Collector → Tempo（链路）+ Prometheus（指标）+ Loki（日志）+ Grafana |
| 📖 | **文档即代码** | OpenAPI 3.0 / Swagger UI / Agent Prompt / SDK 全自动生成 |
| 🖥️ | **Web 管理平台** | React 18 SPA，8 个功能页面覆盖项目管理、文档、沙箱、补偿、Webhook |

---

## 系统架构

### 架构总览

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
           │ 遗留系统         │
           │ SOAP/OData/     │
           │ OpenAPI/CLI/    │
           │ SSH/PTY         │
           └────────────────┘

  ┌─────────────────────────────────────────────────┐
  │     横切关注点 (所有组件共享)                      │
  │  OTel Tracing │ 事件总线 │ 配置热加载              │
  └─────────────────────────────────────────────────┘
```

### Crate 依赖关系

```
                    ┌──────────┐
                    │  common  │ ← 公共类型、配置、错误定义
                    └─────┬────┘
                          │
              ┌───────────┼───────────┐
              │           │           │
         ┌────▼───┐  ┌───▼────┐  ┌──▼──────────┐
         │metadata│  │event-bus│  │ plugin-sdk  │
         └────┬───┘  └───┬────┘  └─────────────┘
              │          │
    ┌─────────┼──────────┼──────────┐
    │         │          │          │
┌───▼────┐┌──▼─────┐┌───▼───┐┌────▼───────┐
│gateway ││generator││sandbox││compensation│
└───┬────┘└────────┘└───────┘└────────────┘
    │
┌───▼─────────┐    ┌─────┐
│platform-api │    │ cli │
└─────────────┘    └─────┘
```

### 网关请求处理管道

```
Request → 动态路由匹配 → 限流检查 → 熔断检查 → 信号量获取
       → ProtocolAdapter.call() → 错误规范化(RFC 7807)
       → 投递记录 → 事件发布 → Response
```

---

## 快速开始

### 环境要求

| 依赖 | 版本 |
|------|------|
| Rust | 1.82+ |
| PostgreSQL | 16+ |
| Node.js | 18+ (前端构建) |
| Podman / Docker | 最新版 |

### 1. 克隆项目

```bash
git clone https://github.com/your-org/api-anything.git
cd api-anything
```

### 2. 启动基础设施

```bash
cd docker
podman compose up -d   # 启动 PostgreSQL + Kafka + OTel + Grafana 全家桶
cd ..
```

服务列表：PostgreSQL (5432) | Kafka (9092) | OTel Collector (4317) | Tempo (3200) | Prometheus (9090) | Loki (3100) | Grafana (3000)

### 3. 配置环境变量

```bash
cp .env.example .env
# 默认配置即可运行，无需修改
```

### 4. 编译并运行

```bash
# 编译全部 crate
cargo build --release

# 运行 Platform API 服务
cargo run --release -p api-anything-platform-api
```

### 5. 第一个 WSDL → REST 转换（LLM 代码生成）

```bash
# 使用 codegen 命令，LLM 驱动全自动生成 Rust 插件
cargo run -p api-anything-cli -- codegen \
  -s docs/test-data/complex-order-service.wsdl \
  -t soap \
  -p my-first-api \
  --workspace ./workspace

# 输出：
#   [Stage 1] LLM analyzing interface semantics...
#   [Stage 2] LLM generating Rust source code...
#   [Stage 3] Compiling plugin... OK (353 lines, 4.9M)
#   [Stage 5] Generating OpenAPI 3.0 spec...
#   [Stage 7] Plugin loaded, 5 routes registered
```

也支持传统的元数据生成模式（不依赖 LLM）：

```bash
cargo run -p api-anything-cli -- generate \
  --source docs/test-data/complex-order-service.wsdl \
  --project my-first-api
```

### 6. 访问 Web 管理平台

```bash
# 构建前端
cd web && npm install && npm run build && cd ..

# 重新启动 API 服务（自动托管 web/dist 静态文件）
cargo run --release -p api-anything-platform-api

# 打开浏览器访问 http://localhost:8080
```

---

## 项目结构

```
api-anything/
├── crates/                    10 个 Rust crate
│   ├── common/                公共类型、配置、错误定义
│   ├── metadata/              PostgreSQL 元数据仓库（契约/路由/绑定 CRUD）
│   ├── generator/             7 阶段生成引擎（WSDL/CLI/SSH 解析 + LLM 映射 + OpenAPI 生成）
│   ├── gateway/               网关运行时（适配器 + 保护层 + 动态路由 + 插件加载）
│   ├── sandbox/               沙箱平台（Mock/Replay/Proxy 三层）
│   ├── compensation/          补偿引擎（重试/幂等/死信/告警）
│   ├── event-bus/             事件总线（PG 默认 + Kafka 可选）
│   ├── plugin-sdk/            插件 SDK（C ABI 接口 + export_plugin! 宏）
│   ├── platform-api/          统一 HTTP 后端（Axum 路由 + 中间件 + 静态文件托管）
│   └── cli/                   命令行工具（codegen / generate / generate-ssh / generate-cli）
│       └── codegen/           LLM 代码生成引擎（7 阶段编排 + prompt 模板 + 编译器 + 脚手架）
├── web/                       React 18 + TypeScript + Vite + TailwindCSS 前端
│   └── src/pages/             8 个功能页面
├── docker/                    Docker Compose 编排（8 个服务）
├── docs/                      文档集（设计规格 + 测试计划 + 用户手册）
├── scripts/                   工具脚本（E2E 测试运行器）
└── test-reports/              测试报告 + 前端截图
```

### Crate 职责速查

| Crate | 职责 |
|-------|------|
| `common` | 公共类型定义（`SourceType`、`ProtocolType`、`DeliveryGuarantee`）、配置加载、错误类型 |
| `metadata` | PostgreSQL 元数据仓库的 `MetadataRepo` trait 和 `PgMetadataRepo` 实现 |
| `generator` | 元数据生成流水线：解析 → 统一建模 → LLM 增强映射 → 持久化 → OpenAPI/Agent Prompt 生成 |
| `cli/codegen` | LLM 代码生成引擎：7 阶段流水线（语义分析 → Rust 源码 → 编译 → 测试 → OpenAPI → 热加载） |
| `gateway` | 在线网关运行时：4 种协议适配器 + 三层保护 + ArcSwap 动态路由 + 错误规范化 |
| `sandbox` | 沙箱测试平台：MockLayer（Schema 驱动）+ ReplayLayer（录制回放）+ ProxyLayer（真实代理） |
| `compensation` | 数据补偿引擎：投递记录 → 指数退避重试 → 幂等去重 → 死信管理 |
| `event-bus` | 事件总线抽象层：`EventBus` trait + PG 轮询实现 + Kafka 可选实现 |
| `plugin-sdk` | 插件开发 SDK：`PluginInfo`/`PluginRequest`/`PluginResponse` 类型 + `export_plugin!` 宏 |
| `platform-api` | 统一 HTTP API 入口：Axum 路由注册、中间件管道、SPA 静态文件兜底 |
| `cli` | 命令行工具：`codegen`（LLM 代码生成）、`generate`（WSDL）、`generate-ssh`（SSH）、`generate-cli`（CLI）四个子命令 |

---

## 核心模块详解

### LLM 驱动代码生成引擎 (codegen)

这是 API-Anything 的核心能力。不同于传统的"确定性规则映射"，代码生成引擎实现了真正的 **LLM 驱动 7 阶段代码生成流水线**，直接从接口定义生成可编译、可运行的 Rust 插件：

```
输入（WSDL/OData/CLI/SSH/PTY）
  → Stage 1: LLM 分析接口语义
  → Stage 2: LLM 生成强类型 Rust 源码（serde struct + ProtocolAdapter）
  → Stage 3: cargo build 编译为 .so/.dylib（失败自动反馈 LLM 修正，最多 5 次）
  → Stage 4: LLM 生成影子测试代码
  → Stage 5: LLM 提取路由信息 → 生成 OpenAPI 3.0
  → Stage 6: 观测注入（代码中包含 #[tracing::instrument]）
  → Stage 7: 产物存储 → PluginManager 热加载 → 网关路由自动注册
```

**核心模块组成：**

| 模块 | 文件 | 职责 |
|------|------|------|
| 编排引擎 | `codegen/mod.rs` | 7 阶段流水线编排，统一入口 |
| Prompt 模板 | `codegen/prompts.rs` | 6 种接口类型的 LLM prompt 模板 |
| 编译器 | `codegen/compiler.rs` | cargo build + 编译失败自动反馈 LLM 修复（最多 5 轮） |
| 脚手架 | `codegen/scaffold.rs` | 临时 crate 脚手架生成（Cargo.toml + lib.rs） |
| 参考代码 | `codegen/reference_plugins.rs` | 6 种接口的可编译参考代码，作为 LLM few-shot 示例 |

**支持的输入类型（6 种协议）：**

| 输入类型 | 解析器 | 解析策略 |
|---------|--------|---------|
| WSDL (SOAP) | `WsdlParser` | quick-xml 结构化解析 portType/binding/service/types |
| OData $metadata | 原生支持 | EntityType/EntitySet/NavigationProperty 解析 |
| OpenAPI 3.0 | 原生支持 | paths/schemas/parameters 解析 |
| CLI --help | `CliHelpParser` | Clap/ArgParse 风格参数识别，支持主命令 + 子命令 |
| SSH 样本 | `SshSampleParser` | 自定义 `## Command:` 块格式解析 |
| PTY 交互 | 原生支持 | Expect 模式 prompt/command/output 解析 |

**LLM 生成代码示例（SOAP Banking TransferFundsRequest）：**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferFundsRequest {
    pub from_account: String,
    pub to_account: String,
    pub amount: f64,
    pub currency: String,
    pub reference: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferFundsResponse {
    pub transaction_id: String,
    pub status: TransferStatus,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransferStatus {
    Completed,
    Pending,
    Failed,
}
```

**LLM 代码生成实测结果（全部通过）：**

| 接口类型 | 测试场景 | 路由数 | 生成代码行数 | 插件大小 |
|---------|---------|--------|------------|---------|
| SOAP | 计算器（简单） | 2 | ~200 行 | .dylib |
| SOAP | 订单系统（复杂嵌套） | 5 | 353 行 | 4.9M |
| SOAP | 银行系统（枚举/decimal） | 6 | 491 行 | .dylib |
| OData | 产品服务（5 EntityType） | 20 | 167 行 | 3.3M |
| OpenAPI | Petstore（CRUD+嵌套） | 7 | 170 行 | 3.3M |
| CLI | 数据库工具 | 10 | ~150 行 | 2.1M |
| CLI | DevOps 工具（8 子命令） | 6 | 154 行 | 2.1M |
| SSH | 网络交换机 | 6 | ~200 行 | 2.2M |
| SSH | 服务器管理 | 7 | ~300 行 | 563K |
| SSH | 企业路由器（BGP/OSPF） | 12 | 447 行 | .dylib |
| PTY | MySQL REPL | 4 | ~200 行 | 2.2M |
| PTY | Redis CLI（5 种数据结构） | 18 | 377 行 | 2.1M |
| **合计** | **12 场景** | **103 路由** | | |

### 多厂商 LLM 支持

代码生成引擎支持 **7 家 LLM 厂商**，通过统一的 Provider 抽象层适配不同 API 格式：

| 厂商 | Provider 值 | 默认模型 | API 格式 |
|------|------------|---------|---------|
| Anthropic | `anthropic` | claude-sonnet-4 | Claude Messages API |
| OpenAI | `openai` | gpt-4o | OpenAI Chat |
| Google | `gemini` | gemini-2.0-flash | Gemini API |
| 智谱 | `glm` | glm-4-flash | OpenAI 兼容 |
| 通义千问 | `qwen` | qwen-max | OpenAI 兼容 |
| 月之暗面 | `kimi` | moonshot-v1-128k | OpenAI 兼容 |
| DeepSeek | `deepseek` | deepseek-chat | OpenAI 兼容 |

### 元数据生成引擎 (generator)

除 LLM 代码生成引擎外，元数据生成引擎仍采用 **7 阶段流水线**，将遗留系统契约自动转换为 REST API 元数据：

```
输入契约 → [1]解析 → [2]统一建模 → [3]LLM增强映射 → [4]持久化
         → [5]后端绑定创建 → [6]路由注册 → [7]OpenAPI/Agent Prompt 生成
```

**LLM 增强映射：** `LlmEnhancedMapper` 调用 Claude/OpenAI API 理解操作语义，为 REST 路由选择合理的 HTTP 方法和 URL 路径。当 LLM 不可用时，`WsdlMapper`/`CliMapper`/`SshMapper` 提供确定性降级方案。

### 网关运行时 (gateway)

网关运行时是在线服务的核心，负责将 REST 请求转发到遗留后端。

**`ProtocolAdapter` trait 统一接口：** 所有协议适配器实现同一个 `call()` 方法，网关调度器无需感知后端协议差异。

**适配器列表：**

| 适配器 | 后端协议 | 实现方式 |
|--------|---------|---------|
| `SoapAdapter` | SOAP/XML | HTTP POST + XML 信封构建 |
| `CliProcessAdapter` | 命令行工具 | `tokio::process` + `.arg()` 安全传参 |
| `SshRemoteAdapter` | SSH 远程命令 | 系统 `ssh` 二进制包装 |
| `PtyExpectAdapter` | PTY 交互程序 | Expect 状态机 stdin/stdout pipe |
| Plugin (.so/.dylib) | SOAP/OData/OpenAPI/CLI/SSH/PTY | LLM 生成 + C ABI `libloading` 动态加载 |

**动态路由：** 使用 `ArcSwap` 实现 RCU（Read-Copy-Update）热加载，路由表更新对读路径零锁开销。

**三层保护（协议感知默认值）：**

| 保护层 | 实现 | 说明 |
|--------|------|------|
| 限流 | `RateLimiter` 令牌桶 | 突发流量平滑，保护脆弱后端 |
| 熔断 | `CircuitBreaker` 滑动窗口 | Closed → Open → HalfOpen 三态转换 |
| 并发 | `ConcurrencySemaphore` | 协议感知默认值：CLI=10, SSH=5 |

### 沙箱平台 (sandbox)

沙箱通过 `/sandbox/*` 路径和 `X-Sandbox-Mode` 请求头激活，三种模式覆盖不同联调阶段：

| 模式 | 说明 |
|------|------|
| **Mock** | Schema 驱动 + Smart Mock 语义字段（email/phone/date 等智能生成）+ Fixed Response |
| **Replay** | 自动录制真实交互，精确匹配或模糊匹配（top-level key 相似度）回放 |
| **Proxy** | 转发真实后端，租户隔离 + 只读模式保护 |

### 补偿引擎 (compensation)

为保护脆弱的遗留后端，补偿引擎提供三级投递保障：

| 级别 | 策略 | 行为 |
|------|------|------|
| `at_most_once` | 最多一次 | 不记录投递日志，火即忘 |
| `at_least_once` | 至少一次 | 记录投递 → 失败指数退避重试（1s→5s→30s→5min→30min） |
| `exactly_once` | 精确一次 | `idempotency_keys` 表 + 重复拒绝 + 重试 |

超过最大重试次数后自动转入 **死信队列**，管理 API 支持查看、单条重推、批量重推、人工标记解决。

### 事件总线 (event-bus)

抽象的 `EventBus` trait 支持两种后端实现：

| 实现 | 场景 |
|------|------|
| `PgEventBus` | 默认，PostgreSQL 轮询，零额外依赖 |
| `KafkaEventBus` | 可选（feature flag），高吞吐场景 |

**事件类型枚举：**

| 事件 | 触发时机 |
|------|---------|
| `RouteUpdated` | 路由表变更 |
| `DeliveryFailed` | 投递失败 |
| `DeliverySucceeded` | 投递成功 |
| `DeadLetter` | 转入死信 |
| `GenerationCompleted` | 生成流水线完成 |
| `CircuitBreakerOpened` | 熔断器打开 |
| `CircuitBreakerClosed` | 熔断器关闭 |

### 插件系统 (plugin-sdk)

插件通过 C ABI 跨 `.so` 边界交互，所有数据经 JSON 序列化传递，避免内存布局差异：

```rust
// 插件开发者只需实现 handler 函数，使用 export_plugin! 宏导出
use api_anything_plugin_sdk::*;

fn my_handler(req: PluginRequest) -> PluginResponse {
    PluginResponse {
        status_code: 200,
        headers: HashMap::new(),
        body: serde_json::json!({"result": "ok"}),
    }
}

export_plugin!(my_handler, PluginInfo {
    name: "my-protocol".into(),
    version: "1.0.0".into(),
    protocol: "custom".into(),
    description: "My custom protocol adapter".into(),
});
```

宏自动生成 `plugin_info()`、`plugin_handle()`、`plugin_free()` 三个 C ABI 导出函数。

---

## Web 管理平台

React 18 + TypeScript + TailwindCSS 构建的 SPA，8 个功能页面：

| 页面 | 截图 | 功能描述 |
|------|------|---------|
| Dashboard | ![Dashboard](test-reports/frontend-screenshots/01-dashboard-project-list.png) | 项目卡片列表 + 创建/删除 CRUD 管理 |
| API Docs | ![API Docs](test-reports/frontend-screenshots/08-docs-page-swagger-embed.png) | Swagger UI 嵌入浏览 + OpenAPI JSON 下载 |
| SDK Generator | ![SDK Generator](test-reports/frontend-screenshots/p6-04-sdk-generator.png) | TypeScript / Python / Java / Go SDK 代码生成 |
| API Explorer | ![API Explorer](test-reports/frontend-screenshots/p6-02-api-explorer.png) | 类 Postman 交互式 API 调试工具 |
| Agent Prompt | ![Agent Prompt](test-reports/frontend-screenshots/09-docs-agent-prompt.png) | AI Agent 可消费的结构化 API 描述 |
| Sandbox | ![Sandbox](test-reports/frontend-screenshots/11-sandbox-project-selected.png) | 沙箱会话管理 + Mock/Replay/Proxy 模式切换 |
| Compensation | ![Compensation](test-reports/frontend-screenshots/12-compensation-dead-letter-queue.png) | 死信队列表格 + 重推 + 标记解决 |
| Webhooks | ![Webhooks](test-reports/frontend-screenshots/p6-03-webhook-manager.png) | Webhook 订阅创建/列表/删除 |

---

## API 端点速查

### 健康检查

| 方法 | 路径 | 说明 |
|------|------|------|
| `GET` | `/health` | 存活检查 |
| `GET` | `/health/ready` | 就绪检查（含数据库连接） |

### 项目管理

| 方法 | 路径 | 说明 |
|------|------|------|
| `POST` | `/api/v1/projects` | 创建项目 |
| `GET` | `/api/v1/projects` | 项目列表 |
| `GET` | `/api/v1/projects/{id}` | 项目详情 |
| `DELETE` | `/api/v1/projects/{id}` | 删除项目 |

### 沙箱会话

| 方法 | 路径 | 说明 |
|------|------|------|
| `POST` | `/api/v1/projects/{project_id}/sandbox-sessions` | 创建沙箱会话 |
| `GET` | `/api/v1/projects/{project_id}/sandbox-sessions` | 会话列表 |
| `DELETE` | `/api/v1/sandbox-sessions/{id}` | 删除会话 |

### 录制数据

| 方法 | 路径 | 说明 |
|------|------|------|
| `GET` | `/api/v1/sandbox-sessions/{id}/recordings` | 列出录制记录 |
| `DELETE` | `/api/v1/sandbox-sessions/{id}/recordings` | 清空录制记录 |

### 补偿系统

| 方法 | 路径 | 说明 |
|------|------|------|
| `GET` | `/api/v1/compensation/dead-letters` | 死信队列列表 |
| `POST` | `/api/v1/compensation/dead-letters/batch-retry` | 批量重推 |
| `POST` | `/api/v1/compensation/dead-letters/{id}/retry` | 单条重推 |
| `POST` | `/api/v1/compensation/dead-letters/{id}/resolve` | 标记解决 |
| `GET` | `/api/v1/compensation/delivery-records/{id}` | 投递记录详情 |

### Webhook 管理

| 方法 | 路径 | 说明 |
|------|------|------|
| `POST` | `/api/v1/webhooks` | 创建订阅 |
| `GET` | `/api/v1/webhooks` | 订阅列表 |
| `DELETE` | `/api/v1/webhooks/{id}` | 删除订阅 |

### 插件管理

| 方法 | 路径 | 说明 |
|------|------|------|
| `GET` | `/api/v1/plugins` | 已加载插件列表 |
| `POST` | `/api/v1/plugins/scan` | 扫描并加载新插件 |

### 文档服务

| 方法 | 路径 | 说明 |
|------|------|------|
| `GET` | `/api/v1/docs` | Swagger UI 页面 |
| `GET` | `/api/v1/docs/openapi.json` | OpenAPI 3.0 JSON 规范 |
| `GET` | `/api/v1/docs/agent-prompt` | AI Agent 提示词 |
| `GET` | `/api/v1/docs/sdk/{language}` | SDK 代码生成（language: typescript/python/java/go） |

### 网关代理

| 方法 | 路径 | 说明 |
|------|------|------|
| `ANY` | `/gw/{*rest}` | 动态路由分发，转发到遗留后端 |
| `ANY` | `/sandbox/{*rest}` | 沙箱路由，根据 `X-Sandbox-Mode` 头走 mock/replay/proxy |

---

## CLI 工具

### codegen — LLM 驱动代码生成（核心命令）

```bash
cargo run -p api-anything-cli -- codegen \
  -s <source-file> \
  -t <interface-type> \
  -p <project-name> \
  --workspace <dir>
```

从接口定义文件出发，通过 LLM 驱动的 7 阶段流水线，直接生成可编译的 Rust 插件（.so/.dylib），并自动注册到网关路由。

**支持的 `interface-type`：** `soap`、`odata`、`openapi`、`cli`、`ssh`、`pty`

### generate — WSDL → REST

```bash
cargo run -p api-anything-cli -- generate \
  --source <path/to/service.wsdl> \
  --project <project-name>
```

从 WSDL 文件解析 SOAP 服务，生成 REST API 路由并输出 OpenAPI 规范文件。

### generate-ssh — SSH 样本 → REST

```bash
cargo run -p api-anything-cli -- generate-ssh \
  --sample <path/to/ssh-sample.txt> \
  --project <project-name>
```

从 SSH 交互样本文件（`## Command:` 块格式）解析命令，生成 REST API 路由。

### generate-cli — CLI Help → REST

```bash
cargo run -p api-anything-cli -- generate-cli \
  --main-help <path/to/main-help.txt> \
  --sub-help <subcmd>:<path/to/sub-help.txt> \
  --program <path/to/cli-binary> \
  --project <project-name>
```

从 CLI 工具的 `--help` 输出解析参数结构，支持主命令 + 多个子命令，生成 REST API 路由。

---

## 配置参考

所有配置通过环境变量注入，参见 `.env.example`：

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `DATABASE_URL` | `postgres://api_anything:api_anything@localhost:5432/api_anything` | PostgreSQL 连接字符串 |
| `EVENT_BUS_TYPE` | `pg` | 事件总线类型：`pg`（PostgreSQL 轮询）或 `kafka` |
| `KAFKA_BROKERS` | `localhost:9092` | Kafka broker 地址（仅 `EVENT_BUS_TYPE=kafka` 时生效） |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | `http://localhost:4317` | OpenTelemetry Collector gRPC 端点 |
| `API_HOST` | `0.0.0.0` | API 服务监听地址 |
| `API_PORT` | `8080` | API 服务监听端口 |
| `ALERT_WEBHOOK_URL` | — | 告警通知 Webhook URL（可选） |
| `ALERT_WEBHOOK_TYPE` | `slack` | 告警类型：`slack` 或 `dingtalk` |
| `PLUGIN_DIR` | `./plugins` | 插件 `.so` 文件扫描目录 |
| `LLM_PROVIDER` | `anthropic` | LLM 厂商：`anthropic`/`openai`/`gemini`/`glm`/`qwen`/`kimi`/`deepseek` |
| `LLM_MODEL` | — | LLM 模型名称（留空则使用厂商默认模型） |
| `LLM_API_KEY` | — | LLM API 密钥（必填，用于代码生成引擎） |
| `LLM_BASE_URL` | — | LLM API 基础 URL（可选，用于自定义端点） |
| `CODEGEN_MAX_FIX_ROUNDS` | `5` | 编译失败自动修复最大轮次 |
| `RUST_LOG` | `api_anything=debug,tower_http=debug` | 日志级别（tracing-subscriber env-filter 格式） |

---

## 测试

### 测试概览

| 指标 | 数值 |
|------|------|
| 自动化测试总数 | **342** |
| 测试通过率 | **100%** |
| 覆盖模块 | 8 个 crate + codegen 子模块 |
| LLM 代码生成场景 | 12 场景 / 103 路由 |
| 前端截图 | 16 张 |

### 模块测试分布

| 模块 | 测试数 | 说明 |
|------|--------|------|
| common | 4 | 配置加载、类型序列化 |
| gateway | 50 | 4 种适配器 + 三层保护 + 错误规范化 + 命令注入防护 |
| generator | 47 | WSDL/CLI/SSH 解析 + LLM 映射 + OpenAPI 生成 |
| codegen | 35 | 6 种协议代码生成 + 编译修复 + prompt 模板 + 脚手架 |
| sandbox | 9 | Mock/Replay/Proxy 三层 + 会话管理 |
| compensation | 6 | 重试策略 + 幂等 + 死信 |
| metadata | 6 | 数据库 CRUD 集成测试 |
| platform-api | 117 | 全部 API 端点 + 中间件 + 网关代理集成 |
| cli | 4 | 四个子命令的端到端测试 |

### 测试覆盖范围

- **协议适配器：** SOAP (WSDL 解析 → XML↔JSON → 网关代理)、OData ($metadata 解析 → REST CRUD)、OpenAPI (paths/schemas 解析)、CLI (help 解析 → tokio::process → 输出解析)、SSH (样例解析 → 系统 ssh → 输出解析)、PTY (Expect 状态机 → stdin/stdout pipe)
- **LLM 代码生成：** 12 个端到端场景全部通过，覆盖 6 种协议类型，累计生成 103 条路由
- **保护层：** 令牌桶限流 (burst → refill → reject)、滑动窗口熔断 (Closed → Open → HalfOpen)、并发信号量 (acquire → release → limit)
- **安全：** 7 种命令注入变体防护、RFC 7807 错误格式、敏感信息泄露检查

### 运行测试

```bash
# 运行全部测试
cargo test

# 运行 E2E 测试并生成报告
./scripts/run-e2e-tests.sh
# 报告输出到 test-reports/<timestamp>/test-report.md
```

---

## 技术栈

### 后端

| 技术 | 版本 | 用途 |
|------|------|------|
| Rust | 1.82+ | 系统语言 |
| Axum | 0.8 | Web 框架 |
| Tokio | 1.x | 异步运行时 |
| SQLx | 0.8 | PostgreSQL 异步驱动 |
| Tower | 0.5 | 中间件抽象层 |
| ArcSwap | 1.x | RCU 动态路由热加载 |
| DashMap | 6.x | 并发 HashMap |
| quick-xml | 0.37 | WSDL/SOAP XML 解析 |
| russh | 0.58 | SSH 客户端库 |
| libloading | 0.8 | 动态库 `.so` 加载 |
| rdkafka | 0.37 | Kafka 客户端 |
| OpenTelemetry | 0.27 | 分布式追踪 |
| Clap | 4.x | CLI 参数解析 |
| Serde | 1.x | 序列化框架 |

### 前端

| 技术 | 用途 |
|------|------|
| React 18 | UI 框架 |
| TypeScript | 类型安全 |
| Vite | 构建工具 |
| TailwindCSS | 样式框架 |
| React Router | SPA 路由 |

### 基础设施

| 组件 | 版本 | 用途 |
|------|------|------|
| PostgreSQL | 16-alpine | 元数据仓库 + 事件总线 |
| Kafka (Confluent) | 7.7.0 | 可选事件总线 |
| OTel Collector | 0.114.0 | 遥测数据收集 |
| Grafana | 11.4.0 | 可视化仪表盘 |
| Tempo | 2.6.1 | 分布式链路追踪 |
| Prometheus | 2.55.1 | 指标存储 |
| Loki | 3.3.2 | 日志聚合 |

---

## 文档索引

| 文档 | 路径 | 说明 |
|------|------|------|
| 平台全局架构设计规格书 | [`docs/superpowers/specs/2026-03-21-api-anything-platform-design.md`](docs/superpowers/specs/2026-03-21-api-anything-platform-design.md) | 完整系统架构、元数据模型、核心决策 |
| Phase 0: 基础设施 | [`docs/superpowers/specs/2026-03-21-phase0-infrastructure.md`](docs/superpowers/specs/2026-03-21-phase0-infrastructure.md) | Workspace 结构、数据库 Schema、Docker Compose |
| Phase 1a: 网关运行时 | [`docs/superpowers/specs/2026-03-21-phase1a-gateway-runtime.md`](docs/superpowers/specs/2026-03-21-phase1a-gateway-runtime.md) | 动态路由、保护层、错误规范化 |
| Phase 1b: WSDL/SOAP 适配 | [`docs/superpowers/specs/2026-03-21-phase1b-wsdl-soap-adapter.md`](docs/superpowers/specs/2026-03-21-phase1b-wsdl-soap-adapter.md) | WSDL 解析、SOAP 适配器、OpenAPI 生成 |
| Phase 1c: LLM 集成 | [`docs/superpowers/specs/2026-03-21-phase1c-live-gateway-llm.md`](docs/superpowers/specs/2026-03-21-phase1c-live-gateway-llm.md) | LLM 增强映射、确定性降级 |
| Phase 2a: CLI 适配器 | [`docs/superpowers/specs/2026-03-21-phase2a-cli-adapter.md`](docs/superpowers/specs/2026-03-21-phase2a-cli-adapter.md) | CLI help 解析、进程适配器 |
| Phase 2b: SSH 适配器 | [`docs/superpowers/specs/2026-03-21-phase2b-ssh-adapter.md`](docs/superpowers/specs/2026-03-21-phase2b-ssh-adapter.md) | SSH 样本解析、远程命令适配器 |
| Phase 3: 沙箱平台 | [`docs/superpowers/specs/2026-03-22-phase3-sandbox.md`](docs/superpowers/specs/2026-03-22-phase3-sandbox.md) | Mock/Replay/Proxy 三层 |
| Phase 4: 补偿引擎 | [`docs/superpowers/specs/2026-03-22-phase4-compensation.md`](docs/superpowers/specs/2026-03-22-phase4-compensation.md) | 重试策略、幂等、死信 |
| Phase 5a: PTY + 文档 | [`docs/superpowers/specs/2026-03-22-phase5a-pty-docs.md`](docs/superpowers/specs/2026-03-22-phase5a-pty-docs.md) | PTY 适配器、Agent Prompt |
| Phase 5b: Web 门户 | [`docs/superpowers/specs/2026-03-22-phase5b-web-portal.md`](docs/superpowers/specs/2026-03-22-phase5b-web-portal.md) | React 前端、8 个功能页面 |
| Phase 6: 功能补完 | [`docs/superpowers/specs/2026-03-22-phase6-completion.md`](docs/superpowers/specs/2026-03-22-phase6-completion.md) | Explorer、SDK、Webhook、插件 |
| 功能完成度报告 | [`docs/feature-completion-report.md`](docs/feature-completion-report.md) | 逐项对照规格书核查 |
| E2E 测试计划 | [`docs/e2e-test-plan.md`](docs/e2e-test-plan.md) | 全链路测试方案 |
| 手动测试手册 | [`docs/manual-testing-handbook.md`](docs/manual-testing-handbook.md) | 手动验证步骤 |
| 用户手册 | [`docs/user-manual.md`](docs/user-manual.md) | 使用指南 |
| 部署指南 | [`docs/deployment-guide.md`](docs/deployment-guide.md) | 部署方案 |

---

## 路线图

### 已完成

| Phase | 内容 | 状态 |
|-------|------|------|
| Phase 0 | 基础设施（Workspace + DB + Docker + OTel + CI） | ✅ 100% |
| Phase 1 | WSDL→REST + 网关运行时 + LLM 增强映射 | ✅ 95% |
| Phase 2 | CLI/SSH 适配器扩展 | ✅ 95% |
| Phase 3 | 沙箱测试平台（Mock/Replay/Proxy） | ✅ 90% |
| Phase 4 | 数据补偿引擎（重试/幂等/死信） | ✅ 85% |
| Phase 5 | Web 门户 + PTY 适配器 | ✅ 80% |
| Phase 6 | Explorer + SDK + Webhook + 插件 | ✅ 100% |
| **Codegen** | **LLM 驱动代码生成引擎（7 阶段流水线 + 7 厂商 + 6 协议 + 12 场景）** | **✅ 100%** |

### 待完成

| 优先级 | 功能 | 说明 |
|--------|------|------|
| P1 | Kafka 事件总线 | 替换 PG 轮询，提升高吞吐场景性能 |
| P1 | Push Dispatcher | Webhook/回调主动推送 |
| P2 | SSH 连接池 | 高频 SSH 场景性能优化 |
| P2 | Grafana 面板嵌入 | 在 Web 前端直接查看监控 |
| P2 | 告警通知集成 | Slack / DingTalk 外部告警 |
| P3 | RAG 大型 WSDL 分块 | 超大 WSDL 文件智能分块处理 |
| P3 | TLS 双向认证 | 生产环境安全加固 |
| P3 | JWT 认证 | API 访问控制 |
| P4 | Kubernetes Helm Chart | K8s 一键部署 |
| P4 | Contract Diff | 版本变更日志 + Breaking Change 检测 |

---

## License

[MIT](LICENSE)
