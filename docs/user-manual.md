# API-Anything 用户使用手册

## 目录

- [1. 平台概述](#1-平台概述)
- [2. 快速开始](#2-快速开始)
- [3. CLI 工具使用](#3-cli-工具使用)
  - [3.1 WSDL/SOAP 接口转换](#31-wsdlsoap-接口转换)
  - [3.2 CLI 命令行工具包装](#32-cli-命令行工具包装)
  - [3.3 SSH 远程命令包装](#33-ssh-远程命令包装)
- [4. Web 管理平台](#4-web-管理平台)
  - [4.1 仪表盘 (Dashboard)](#41-仪表盘-dashboard)
  - [4.2 API 文档](#42-api-文档)
  - [4.3 沙箱测试管理](#43-沙箱测试管理)
  - [4.4 补偿管理](#44-补偿管理)
  - [4.5 API Explorer](#45-api-explorer)
  - [4.6 Webhook 管理](#46-webhook-管理)
  - [4.7 SDK 代码生成](#47-sdk-代码生成)
  - [4.8 Monitoring（监控面板）](#48-monitoring监控面板)
- [5. 网关使用](#5-网关使用)
  - [5.1 通过网关调用转换后的 API](#51-通过网关调用转换后的-api)
  - [5.2 投递保障配置](#52-投递保障配置)
  - [5.3 沙箱调用](#53-沙箱调用)
  - [5.4 Webhook 推送](#54-webhook-推送)
  - [5.5 告警配置](#55-告警配置)
- [6. 监控与可观测性](#6-监控与可观测性)
- [7. LLM 增强功能](#7-llm-增强功能)
- [8. 常见问题 (FAQ)](#8-常见问题-faq)
- [9. 安全配置](#9-安全配置)
  - [9.1 TLS/HTTPS 配置](#91-tlshttps-配置)
  - [9.2 JWT 认证配置](#92-jwt-认证配置)
  - [9.3 敏感数据加密](#93-敏感数据加密)
- [10. 运维配置](#10-运维配置)
  - [10.1 路由热加载](#101-路由热加载)
  - [10.2 WebSocket 实时推送](#102-websocket-实时推送)
  - [10.3 OTel 自定义指标](#103-otel-自定义指标)

---

## 1. 平台概述

![Swagger UI 展示所有网关路由](test-reports/frontend-screenshots/06-swagger-ui-loaded.png)
*图 1.1: API-Anything 自动生成的 Swagger UI — 展示从 WSDL 转换而来的 REST API 路由*

### API-Anything 是什么

API-Anything 是一个 AI 驱动的遗留系统 API 网关生成平台。它能够自动将 SOAP/WSDL 服务、CLI 命令行工具、SSH 远程命令等传统接口转换为标准的 RESTful JSON API，并提供统一的网关层进行流量管理、投递保障和可观测性。

### 核心能力

| 来源类型 | 输入 | 转换结果 |
|---------|------|---------|
| **SOAP/WSDL** | `.wsdl` 文件 | JSON REST API (SOAP 协议转发) |
| **CLI** | 命令行工具 `--help` 输出 | JSON REST API (子进程执行) |
| **SSH** | SSH 交互样本文件 | JSON REST API (SSH 远程执行) |
| **PTY** | 终端交互录制 | JSON REST API (PTY 会话) |

转换流程统一为五个阶段：

1. **解析 (Parse)** — 从源文件中提取接口结构定义
2. **映射 (Map)** — 将接口定义映射为统一合约中间表示 (UnifiedContract)
3. **持久化 (Persist)** — 将合约、路由、后端绑定写入 PostgreSQL
4. **注册路由 (Register)** — 在网关动态路由表中注册 API 端点
5. **生成规范 (Generate)** — 输出 OpenAPI 3.0 JSON 规范文件

### 系统架构简介

```
┌──────────────┐    ┌──────────────────┐    ┌────────────────────┐
│  CLI 工具     │    │  Web 管理平台     │    │  下游开发者/Agent   │
│ (api-anything)│    │  (React SPA)     │    │  (HTTP Client)     │
└──────┬───────┘    └────────┬─────────┘    └────────┬───────────┘
       │                     │                       │
       │   generate/         │   /api/v1/*           │   /gw/*
       │   generate-ssh/     │                       │   /sandbox/*
       │   generate-cli      │                       │
       ▼                     ▼                       ▼
┌─────────────────────────────────────────────────────────────────┐
│                    Platform API (Axum)                          │
│  ┌──────────┐ ┌───────────┐ ┌───────────┐ ┌────────────────┐  │
│  │ 项目管理  │ │ 沙箱管理   │ │ 补偿管理   │ │ 网关 Handler   │  │
│  └──────────┘ └───────────┘ └───────────┘ └────────┬───────┘  │
│                                                     │          │
│  ┌──────────────────────┐  ┌─────────────────────┐  │          │
│  │ DynamicRouter        │  │ BackendDispatcher    │◄─┘          │
│  │ (路由匹配)           │  │ (限流→熔断→超时→分发) │             │
│  └──────────────────────┘  └──────────┬──────────┘             │
└────────────────────────────────────────┼───────────────────────┘
                                         │
                    ┌────────────────────┼────────────────────┐
                    ▼                    ▼                    ▼
             ┌────────────┐      ┌────────────┐      ┌────────────┐
             │ SOAP 后端   │      │ CLI 执行    │      │ SSH 远程   │
             │ (HTTP/XML)  │      │ (子进程)    │      │ (SSH 连接)  │
             └────────────┘      └────────────┘      └────────────┘
```

基础设施组件：

- **PostgreSQL 16** — 元数据存储（项目、合约、路由、投递记录等）
- **Kafka** — 异步事件总线
- **OpenTelemetry Collector** — 遥测数据收集（Traces/Metrics/Logs）
- **Grafana / Prometheus / Tempo / Loki** — 可观测性全家桶

### 角色和使用场景

| 角色 | 职责 | 主要工具 |
|------|------|---------|
| **平台管理员** | 部署平台、创建项目、管理路由、处理死信、监控健康 | CLI 工具、Web 管理平台、Grafana |
| **下游开发者** | 通过网关调用转换后的 REST API、使用沙箱联调 | HTTP 客户端、Swagger UI、Agent Prompt |

---

## 2. 快速开始

本节以将一个 WSDL 计算器服务转换为 REST API 为例，演示完整流程。

### 前置条件

1. 已启动基础设施服务：

```bash
cd docker
docker compose up -d
```

2. 确认服务就绪：

```bash
# 检查数据库
docker compose ps postgres

# 检查 Platform API 健康状态（需先启动 Platform API）
curl http://localhost:8080/health
# 预期响应: {"status":"ok"}

curl http://localhost:8080/health/ready
# 预期响应: {"status":"ready","db":"connected"}
```

### 5 分钟完成第一个 WSDL 转换

**Step 1:** 准备 WSDL 文件。项目自带示例文件 `crates/generator/tests/fixtures/calculator.wsdl`，内容定义了两个操作：`Add`（加法运算）和 `GetHistory`（查询历史记录）。

**Step 2:** 执行生成命令：

```bash
api-anything generate \
  --source crates/generator/tests/fixtures/calculator.wsdl \
  --project calculator-service
```

**Step 3:** 查看输出：

```
Generation complete!
  Contract ID: a1b2c3d4-...
  Routes created: 2
  OpenAPI spec: crates/generator/tests/fixtures/calculator.wsdl.openapi.json
```

生成流程会：
- 在数据库中创建项目 `calculator-service`（类型为 `wsdl`，所有者为 `cli`）
- 解析 WSDL 并生成两条 REST 路由（对应 `Add` 和 `GetHistory` 操作）
- 在源文件同目录输出 OpenAPI 3.0 规范文件

**Step 4:** 启动 Platform API 后，通过网关调用转换后的 API：

```bash
# 调用 Add 操作
curl -X POST http://localhost:8080/gw/calculator/add \
  -H "Content-Type: application/json" \
  -d '{"a": 5, "b": 3}'

# 调用 GetHistory 操作
curl -X POST http://localhost:8080/gw/calculator/get-history \
  -H "Content-Type: application/json" \
  -d '{"limit": 10}'
```

**Step 5:** 在浏览器中查看 API 文档：

- Swagger UI: http://localhost:8080/api/v1/docs
- OpenAPI JSON: http://localhost:8080/api/v1/docs/openapi.json
- Web 管理平台: http://localhost:8080/

---

## 3. CLI 工具使用

`api-anything` 命令行工具提供三个子命令，分别用于不同来源类型的接口转换。

### 3.1 WSDL/SOAP 接口转换

#### 命令格式

```bash
api-anything generate --source <wsdl-file> --project <name>
```

| 参数 | 必填 | 说明 |
|------|------|------|
| `--source` / `-s` | 是 | WSDL 文件路径 |
| `--project` / `-p` | 是 | 项目名称，在平台中唯一标识该接口集合 |

#### 输入要求

WSDL 文件须为标准的 WSDL 1.1 格式，包含以下核心元素：

- `<types>` — XML Schema 类型定义（请求和响应的结构）
- `<message>` — 消息定义（关联到类型）
- `<portType>` — 操作定义（输入消息和输出消息）
- `<binding>` — SOAP 绑定（soapAction 等）
- `<service>` — 服务端点地址

#### 输出产物

| 产物 | 位置 | 说明 |
|------|------|------|
| 项目记录 | PostgreSQL `projects` 表 | `source_type = wsdl`，`owner = cli` |
| 合约记录 | PostgreSQL `contracts` 表 | 含原始 WSDL 文本和解析后的统一合约 JSON |
| 路由记录 | PostgreSQL `routes` 表 | 每个 SOAP 操作对应一条路由 |
| 后端绑定 | PostgreSQL `backend_bindings` 表 | 含 SOAP endpoint URL、soapAction、namespace 等 |
| OpenAPI 规范 | `<source>.openapi.json` | OpenAPI 3.0 JSON 格式，可导入 Postman 或 Swagger UI |

#### 完整示例

使用项目自带的 `calculator.wsdl` 文件（定义了两个操作 `Add` 和 `GetHistory`）：

```bash
api-anything generate \
  --source crates/generator/tests/fixtures/calculator.wsdl \
  --project my-calculator
```

输出：

```
Generation complete!
  Contract ID: 550e8400-e29b-41d4-a716-446655440000
  Routes created: 2
  OpenAPI spec: crates/generator/tests/fixtures/calculator.wsdl.openapi.json
```

生成的路由映射关系：

| SOAP 操作 | REST 端点 | 说明 |
|-----------|----------|------|
| `Add` | `POST /calculator/add` | 接收 `{"a": int, "b": int}`，返回 `{"result": int}` |
| `GetHistory` | `POST /calculator/get-history` | 接收 `{"limit": int}`，返回 `{"entries": [string]}` |

### 3.2 CLI 命令行工具包装

#### 命令格式

```bash
api-anything generate-cli \
  --main-help <file> \
  --sub-help <name:file> \
  --project <name> \
  --program <path>
```

| 参数 | 必填 | 说明 |
|------|------|------|
| `--main-help` | 是 | 主帮助输出文本文件路径（`command --help` 的输出） |
| `--sub-help` | 否 | 子命令帮助文件，格式为 `子命令名:文件路径`，可重复多次 |
| `--project` / `-p` | 是 | 项目名称 |
| `--program` | 是 | CLI 可执行文件路径（运行时网关通过此路径调用命令） |

#### 如何准备 help 输出文件

1. 获取主帮助文本：

```bash
your-tool --help > main_help.txt
```

2. 获取各子命令的帮助文本：

```bash
your-tool generate --help > generate_help.txt
your-tool list --help > list_help.txt
your-tool export --help > export_help.txt
```

帮助文本需包含以下信息（解析器会自动提取）：

- **SUBCOMMANDS** 区段 — 列出所有子命令及描述
- **OPTIONS** 区段 — 各选项的短名/长名、值类型、默认值、可选值
- **USAGE** 行 — 使用格式

#### 输出格式

每个子命令生成一条 REST 路由，后端绑定的 `endpoint_config` 包含运行时参数：

```json
{
  "program": "/path/to/your-tool",
  "subcommand": "generate",
  "output_format": "json"
}
```

网关在收到请求时，会将 JSON 请求体中的字段转换为命令行参数，执行子进程，并将标准输出解析为 JSON 返回给调用方。

#### 完整示例

以 `report-gen` 报表生成工具为例（项目自带 fixture 文件）：

```bash
api-anything generate-cli \
  --main-help crates/generator/tests/fixtures/sample_help.txt \
  --sub-help generate:crates/generator/tests/fixtures/sample_subcommand_help.txt \
  --project report-service \
  --program /usr/local/bin/report-gen
```

其中 `sample_help.txt` 的内容如下：

```
report-gen 1.2.0
Report generation tool

USAGE:
    report-gen <SUBCOMMAND>

SUBCOMMANDS:
    generate    Generate a new report
    list        List existing reports
    export      Export report to file

OPTIONS:
    -h, --help       Print help information
    -V, --version    Print version
```

`sample_subcommand_help.txt` 的内容如下：

```
report-gen generate
Generate a new report

USAGE:
    report-gen generate [OPTIONS] --type <TYPE>

OPTIONS:
    -t, --type <TYPE>        Report type (daily, weekly, monthly)
    -f, --format <FORMAT>    Output format [default: json] [possible values: json, csv, html]
    -s, --start <DATE>       Start date (YYYY-MM-DD)
    -e, --end <DATE>         End date (YYYY-MM-DD)
    -o, --output <PATH>      Output file path
    -h, --help               Print help information
```

输出：

```
CLI Generation complete!
  Contract ID: 660e8400-...
  Routes created: 3
  OpenAPI spec: crates/generator/tests/fixtures/sample_help.txt.openapi.json
```

生成的路由映射关系：

| 子命令 | REST 端点 | HTTP 方法 |
|--------|----------|----------|
| `generate` | `/report-gen/generate` | POST |
| `list` | `/report-gen/list` | GET |
| `export` | `/report-gen/export` | POST |

### 3.3 SSH 远程命令包装

#### 命令格式

```bash
api-anything generate-ssh --sample <file> --project <name>
```

| 参数 | 必填 | 说明 |
|------|------|------|
| `--sample` | 是 | SSH 交互样例文件路径 |
| `--project` / `-p` | 是 | 项目名称 |

#### SSH 交互样例文件格式

样例文件使用 Markdown 风格的标记语法，定义 SSH 连接参数和命令列表：

```
# SSH Interaction Sample
# Host: <SSH 目标主机>
# User: <SSH 用户名>
# Description: <服务描述>

## Command: <命令模板>
## Description: <命令描述>
## Output Format: <table|text|json>
## Sample Output:
<示例输出内容...>
```

**命令模板** 中可使用 `{参数名}` 占位符表示动态参数。例如 `show running-config interface {interface}` 表示 `interface` 是一个由调用方传入的参数。

**Output Format** 决定网关如何解析命令输出：
- `table` — 表格格式，自动解析为 JSON 数组
- `text` — 纯文本，原样返回
- `json` — JSON 格式，直接透传

#### 完整示例

以网络交换机管理命令为例（项目自带 fixture 文件 `crates/generator/tests/fixtures/ssh_sample.txt`）：

```bash
api-anything generate-ssh \
  --sample crates/generator/tests/fixtures/ssh_sample.txt \
  --project network-switch
```

样例文件内容：

```
# SSH Interaction Sample
# Host: 10.0.1.50
# User: admin
# Description: Network switch management commands

## Command: show interfaces status
## Description: List all network interfaces with their status
## Output Format: table
## Sample Output:
Port      Name     Status    Speed    Duplex
Gi0/1     uplink   connected 1000M    full
Gi0/2     server1  connected 1000M    full
Gi0/3     -        notconnect auto    auto

## Command: show vlan brief
## Description: List VLAN configuration
## Output Format: table
## Sample Output:
VLAN  Name           Status    Ports
1     default        active    Gi0/1, Gi0/2
10    management     active    Gi0/3
20    production     active    Gi0/4, Gi0/5

## Command: show running-config interface {interface}
## Description: Show configuration for a specific interface
## Output Format: text
## Sample Output:
interface GigabitEthernet0/1
 description uplink
 switchport mode trunk
 speed 1000
 duplex full
```

输出：

```
SSH Generation complete!
  Contract ID: 770e8400-...
  Routes created: 3
  OpenAPI spec: crates/generator/tests/fixtures/ssh_sample.txt.openapi.json
```

生成的路由映射关系：

| SSH 命令 | REST 端点 | HTTP 方法 |
|---------|----------|----------|
| `show interfaces status` | `GET /network-switch/interfaces-status` | GET |
| `show vlan brief` | `GET /network-switch/vlan-brief` | GET |
| `show running-config interface {interface}` | `GET /network-switch/running-config-interface/{interface}` | GET |

---

## 4. Web 管理平台

Web 管理平台是一个 React SPA 应用，默认通过 Platform API 的静态文件托管在 `http://localhost:8080/` 上。也可通过 Docker 独立部署在 `http://localhost:3001/`。

平台提供四个主要页面，通过左侧导航栏切换：

| 路径 | 页面 | 功能 |
|------|------|------|
| `/` | Dashboard | 项目列表管理 |
| `/docs` | API Documentation | Swagger UI / Agent Prompt / OpenAPI 下载 |
| `/sandbox` | Sandbox Manager | 沙箱测试会话管理 |
| `/compensation` | Dead Letter Queue | 死信队列与补偿管理 |
| `/explorer` | API Explorer | 交互式 API 调试器 |
| `/webhooks` | Webhook Manager | Webhook 推送订阅管理 |
| `/monitoring` | Monitoring | Grafana 监控面板嵌入 |

### 4.1 仪表盘 (Dashboard)

Dashboard 是平台的首页，展示所有项目的卡片式列表。

![Dashboard 项目列表](test-reports/frontend-screenshots/01-dashboard-project-list.png)
*图 4.1: Dashboard 项目列表 — 展示所有项目卡片，包含项目名称、协议标签（wsdl/cli/ssh）、描述和所有者信息*

#### 项目列表

进入 Dashboard 后会自动加载所有项目。每个项目卡片显示：

- **名称** — 项目名称
- **来源类型标签** — `wsdl` / `cli` / `ssh` / `pty`，以蓝色标签展示
- **描述** — 项目描述，未填写时显示 "No description"
- **所有者** — 创建者或负责团队
- **操作** — 删除按钮

#### 创建新项目

![创建项目表单](test-reports/frontend-screenshots/02-dashboard-create-project-form.png)
*图 4.2: 点击 New Project 后弹出的创建表单*

![表单填写完成](test-reports/frontend-screenshots/03-dashboard-create-project-filled.png)
*图 4.3: 填写完成的项目表单（名称、所有者、描述、协议类型）*

1. 点击右上角 **New Project** 按钮
2. 填写项目表单：

| 字段 | 必填 | 说明 |
|------|------|------|
| Name | 是 | 项目名称 |
| Owner | 是 | 所有者/团队名称 |
| Description | 否 | 项目描述 |
| Source Type | 是 | 来源类型，下拉选择：`WSDL` / `CLI` / `SSH` / `PTY` |

3. 点击 **Create** 提交

![项目创建成功](test-reports/frontend-screenshots/04-dashboard-project-created.png)
*图 4.4: 项目创建成功后，新项目出现在列表顶部*

通过 Web 创建的项目主要用于管理和组织，实际的路由生成仍需通过 CLI 工具执行。

#### 查看项目详情

API 支持按 ID 查询单个项目的详细信息：

```bash
curl http://localhost:8080/api/v1/projects/{project-id}
```

返回项目完整信息，包括 `id`、`name`、`description`、`owner`、`source_type`、`source_config`、`created_at`、`updated_at`。

#### 删除项目

点击项目卡片右下角的 **Delete** 链接。系统会弹出确认对话框，确认后执行删除。删除操作不可逆。

### 4.2 API 文档

API 文档页面提供三种方式查看和使用生成的 API 规范。

#### Swagger UI 在线浏览

![Swagger UI 嵌入](test-reports/frontend-screenshots/08-docs-page-swagger-embed.png)
*图 4.5: Swagger UI 嵌入 Web Portal — 展示 API-Anything Gateway 标题、OAS 3.0 标识和所有 SOAP 路由列表*

切换到 **Swagger UI** 标签页（默认标签），页面会嵌入一个 iframe 加载 `/api/v1/docs` 端点。Swagger UI 从 CDN 加载，自动读取 `/api/v1/docs/openapi.json` 规范。

你可以在 Swagger UI 中：

- 按协议类型（Soap/Http/Cli/Ssh 等）分组浏览 API
- 查看每个端点的请求体 Schema 和响应 Schema
- 直接在浏览器中发起测试请求（Try it out）

OpenAPI 规范动态生成，每次访问都从数据库中的激活路由实时构建，确保文档与实际路由始终一致。

#### Agent Prompt 查看

![Agent Prompt](test-reports/frontend-screenshots/09-docs-agent-prompt.png)
*图 4.6: Agent Prompt 视图 — 展示 Markdown 格式的 API 描述，包含每个路由的方法、路径、协议和 JSON Schema*

切换到 **Agent Prompt** 标签页，页面会加载 `/api/v1/docs/agent-prompt` 端点返回的 Markdown 格式提示词。

该提示词专为 AI Agent（如 ChatGPT / Claude）设计，包含所有激活路由的：
- HTTP 方法和路径
- 协议类型
- 请求体 JSON Schema
- 响应体 JSON Schema

可直接复制该提示词到 AI Agent 的系统提示中，使 Agent 能够自主调用网关 API。

#### OpenAPI JSON 下载

点击右上角 **Download OpenAPI JSON** 按钮，直接下载 `/api/v1/docs/openapi.json` 文件。该文件可导入：

- Postman（Import → OpenAPI 3.0）
- Insomnia
- 各语言的 SDK 生成器（openapi-generator）

### 4.3 沙箱测试管理

沙箱管理页面用于创建和管理沙箱测试会话。

![Sandbox Manager](test-reports/frontend-screenshots/10-sandbox-manager-page.png)
*图 4.7: 沙箱管理页面初始状态 — 需先从下拉列表中选择项目*

#### 选择项目

页面顶部提供项目下拉选择器，需先选择一个项目才能查看和管理该项目下的沙箱会话。

![项目选中后](test-reports/frontend-screenshots/11-sandbox-project-selected.png)
*图 4.8: 选中 demo-soap-calculator 项目后，显示会话列表和 New Session 按钮*

#### 创建沙箱会话

1. 选择项目后，点击 **New Session** 按钮
2. 填写会话表单：

| 字段 | 必填 | 说明 |
|------|------|------|
| Tenant ID | 是 | 租户标识，用于数据隔离 |
| Mode | 是 | 沙箱模式：`Mock` / `Replay` / `Proxy` |
| Expires in hours | 是 | 会话有效期（小时），默认 24 |
| Config JSON | 否 | 会话配置 JSON，默认 `{}` |

3. 点击 **Create** 提交

#### 三种沙箱模式说明

**Mock 模式** — 模拟数据联调

- 不与真实后端通信，根据路由的 `response_schema` 自动生成符合结构的模拟数据
- 适用于前端开发联调、接口契约验证
- 可通过 `config` 字段的 `fixed_response` 配置返回固定数据
- 不要求 `X-Sandbox-Session` 头（可选），未传入时使用纯 schema 推断

**Replay 模式** — 录制回放回归测试

- 根据会话中录制的请求-响应对（`recorded_interactions` 表）进行匹配回放
- 适用于回归测试、接口兼容性验证
- 必须传入 `X-Sandbox-Session` 头
- 请求体会与录制的请求进行匹配，返回对应的录制响应

**Proxy 模式** — 端到端真实验证

- 请求经过完整的网关分发流程，转发到真实后端
- 适用于集成测试、端到端验证
- 必须传入 `X-Sandbox-Session` 头
- 会话配置中可指定目标后端地址等代理参数

#### 使用 Mock 模式联调

创建 Mock 会话后，通过 `/sandbox/*` 前缀发送请求：

```bash
curl -X POST http://localhost:8080/sandbox/calculator/add \
  -H "Content-Type: application/json" \
  -H "X-Sandbox-Mode: mock" \
  -d '{"a": 5, "b": 3}'
```

返回根据 `response_schema` 自动生成的模拟数据。

如需固定响应内容，在创建会话时指定 `config`：

```json
{
  "fixed_response": {
    "result": 42
  }
}
```

然后带上会话 ID 请求：

```bash
curl -X POST http://localhost:8080/sandbox/calculator/add \
  -H "Content-Type: application/json" \
  -H "X-Sandbox-Mode: mock" \
  -H "X-Sandbox-Session: <session-id>" \
  -d '{"a": 5, "b": 3}'
```

#### 使用 Replay 模式回归测试

```bash
curl -X POST http://localhost:8080/sandbox/calculator/add \
  -H "Content-Type: application/json" \
  -H "X-Sandbox-Mode: replay" \
  -H "X-Sandbox-Session: <session-id>" \
  -d '{"a": 5, "b": 3}'
```

#### 使用 Proxy 模式端到端验证

```bash
curl -X POST http://localhost:8080/sandbox/calculator/add \
  -H "Content-Type: application/json" \
  -H "X-Sandbox-Mode: proxy" \
  -H "X-Sandbox-Session: <session-id>" \
  -d '{"a": 5, "b": 3}'
```

#### cURL 示例

每个沙箱会话卡片上都提供了 **cURL Example** 折叠面板，展开后可直接复制包含正确 `X-Sandbox-Mode` 和 `X-Sandbox-Session` 头的完整 cURL 命令。

### 4.4 补偿管理

补偿管理页面用于查看和处理死信队列中的失败投递记录。

![Dead Letter Queue](test-reports/frontend-screenshots/12-compensation-dead-letter-queue.png)
*图 4.9: 死信队列管理界面 — 表格展示 ID、Route、Status、Retries、Error、Updated 等列，支持全选、单条重推和标记已处理*

#### 死信队列查看

进入页面后自动加载最近 50 条死信记录。列表以表格形式展示：

| 列 | 说明 |
|----|------|
| 复选框 | 用于批量操作的多选 |
| ID | 记录 ID（点击可展开查看请求体详情） |
| Route | 关联的路由 ID |
| Status | 当前状态（`dead` 表示已超出最大重试次数） |
| Retries | 已重试次数 |
| Error | 错误信息摘要 |
| Updated | 最后更新时间 |
| Actions | 操作按钮 |

点击 **Refresh** 按钮可手动刷新列表。

#### 手动重推

每条死信记录右侧提供 **Retry** 按钮。点击后系统会将该记录状态重置为 `Failed` 并设置 `next_retry_at` 为当前时间，触发重试 Worker 立即重新处理。

API 等价操作：

```bash
curl -X POST http://localhost:8080/api/v1/compensation/dead-letters/{id}/retry
```

#### 批量重推

1. 使用复选框勾选需要重推的记录（表头的全选复选框可一键选中/取消全部）
2. 点击顶部出现的 **Retry Selected (N)** 按钮
3. 系统会批量重置选中的记录，并返回成功重置的数量

API 等价操作：

```bash
curl -X POST http://localhost:8080/api/v1/compensation/dead-letters/batch-retry \
  -H "Content-Type: application/json" \
  -d '{"ids": ["uuid-1", "uuid-2", "uuid-3"]}'

# 响应: {"retried": 3}
```

批量重试设计为部分容错：单条记录重置失败不会中断整批操作。

#### 标记已处理

若某条死信已通过其他渠道完成投递（例如手动在后端系统中操作），可点击 **Resolve** 按钮将其标记为已人工解决，使其不再出现在死信队列中。

API 等价操作：

```bash
curl -X POST http://localhost:8080/api/v1/compensation/dead-letters/{id}/resolve
```

### 4.5 API Explorer

API Explorer 是一个类 Postman 的交互式 API 调试器，内置于 Web 管理平台中，无需离开浏览器即可对网关路由发起测试请求。

![API Explorer](test-reports/frontend-screenshots/p6-02-api-explorer.png)
*图 4.10: API Explorer — 左侧路由列表，右侧请求构建器与响应查看器*

#### 使用方式

1. 从左侧路由列表中选择目标路由（按项目和协议类型分组）
2. 在右侧面板中构建请求：编辑请求体 JSON、设置请求头
3. 切换目标环境：**Gateway**（`/gw/` 前缀，转发到真实后端）或 **Sandbox**（`/sandbox/` 前缀，走沙箱模式）
4. 点击 **Send** 发送请求
5. 在下方响应区域查看 HTTP 状态码、响应头和响应体

Explorer 会根据路由的 `request_schema` 自动填充请求体模板，减少手动输入。

### 4.6 Webhook 管理

Webhook 管理页面用于创建和管理 Webhook 推送订阅。当平台发生特定事件时，系统会向已订阅的 URL 发送 HTTP POST 通知。

![Webhook Manager](test-reports/frontend-screenshots/p6-03-webhook-manager.png)
*图 4.11: Webhook Manager — 订阅列表与创建表单*

#### 创建 Webhook 订阅

1. 点击 **New Webhook** 按钮
2. 填写订阅信息：

| 字段 | 必填 | 说明 |
|------|------|------|
| URL | 是 | 推送目标地址，需为 HTTPS 或 HTTP 端点 |
| Event Types | 是 | 订阅的事件类型，可多选 |
| Description | 否 | 订阅描述，便于识别用途 |

3. 点击 **Create** 提交

#### 支持的事件类型

| 事件类型 | 说明 |
|---------|------|
| `DeliveryFailed` | 投递失败（进入重试） |
| `DeadLetter` | 死信产生（超出最大重试次数） |
| `GenerationCompleted` | 合约生成完成 |
| `RouteUpdated` | 路由配置变更 |

#### 管理操作

- **列表查看** — 进入页面后自动加载所有 Webhook 订阅，展示 URL、事件类型、描述等信息
- **删除** — 点击订阅行的 **Delete** 按钮移除订阅

API 等价操作：

```bash
# 创建订阅
curl -X POST http://localhost:8080/api/v1/webhooks \
  -H "Content-Type: application/json" \
  -d '{"url":"https://example.com/hook","event_types":["DeadLetter","DeliveryFailed"],"description":"告警通知"}'

# 列出订阅
curl http://localhost:8080/api/v1/webhooks

# 删除订阅
curl -X DELETE http://localhost:8080/api/v1/webhooks/{id}
```

### 4.7 SDK 代码生成

SDK 代码生成功能可以根据当前网关路由的 OpenAPI 规范，自动生成多种语言的 API 客户端代码。

![SDK Generator](test-reports/frontend-screenshots/p6-04-sdk-generator.png)
*图 4.12: SDK Generator — 选择语言后预览生成的客户端代码*

#### 支持语言

| 语言 | 生成内容 |
|------|---------|
| **TypeScript** | 基于 `fetch` 的类型安全客户端 |
| **Python** | 基于 `requests` 的客户端类 |
| **Java** | 基于 `HttpClient` 的客户端类 |
| **Go** | 基于 `net/http` 的客户端包 |

#### 使用方式

1. 进入 API Docs 页面（`/docs`）
2. 切换到 **SDK Generator** 标签页
3. 从语言下拉列表中选择目标语言
4. 页面自动加载并展示生成的客户端代码预览
5. 点击 **Download** 按钮下载代码文件

API 等价操作：

```bash
# 获取 TypeScript SDK
curl http://localhost:8080/api/v1/docs/sdk/typescript

# 获取 Python SDK
curl http://localhost:8080/api/v1/docs/sdk/python
```

### 4.8 Monitoring（监控面板）

Monitoring 页面将 Grafana 仪表盘嵌入到 Web 管理平台中，提供一站式的系统运行状态监控。

#### 面板内容

| 面板 | 指标 | 说明 |
|------|------|------|
| QPS | 每秒请求数 | 实时网关吞吐量 |
| Latency P99 | 第 99 百分位延迟 | 反映长尾请求性能 |
| Error Rate | 5xx 错误率 | 系统健康度核心指标 |
| Circuit Breaker Status | 熔断器状态 | 各后端绑定的熔断器开合状态 |
| Backend Latency | 后端响应延迟 | 按协议类型（SOAP/CLI/SSH）分组 |

#### 前置条件

Monitoring 页面依赖 Grafana 服务运行。请确保通过 Docker Compose 启动了完整的可观测性栈：

```bash
cd docker
docker compose up -d grafana prometheus
```

Grafana 默认地址为 `http://localhost:3000`，Monitoring 页面通过 iframe 嵌入 Grafana 仪表盘。若 Grafana 未启动，页面会显示连接失败提示。

---

## 5. 网关使用

### 5.1 通过网关调用转换后的 API

所有经过 CLI 工具生成的 REST API 端点统一挂载在 `/gw/` 前缀下。

#### 请求格式

- **Content-Type:** `application/json`
- **请求体:** JSON 格式，字段对应原始接口的输入参数
- **请求体大小限制:** 10 MB

```bash
curl -X <METHOD> http://localhost:8080/gw/<path> \
  -H "Content-Type: application/json" \
  -d '<JSON body>'
```

#### 路由匹配规则

网关收到 `/gw/*` 请求后：

1. 去掉 `/gw` 前缀
2. 使用 DynamicRouter 进行路由匹配（支持路径参数，如 `/users/{id}`）
3. 匹配成功后通过 BackendDispatcher 分发到对应的后端（SOAP/CLI/SSH）
4. 后端响应经过转换后以 JSON 格式返回

路由匹配失败时返回 `404 Not Found`。

#### 响应格式

成功响应：

```json
{
  "result": 8
}
```

网关标准错误码：

| HTTP 状态码 | 说明 |
|------------|------|
| 200 | 成功 |
| 404 | 路由不存在 |
| 429 | 请求被限流 |
| 502 | 后端错误 |
| 503 | 熔断器开启 |
| 504 | 后端超时 |

#### 分布式追踪

请求可携带标准的 `traceparent` 请求头（W3C Trace Context 格式），网关会将其传递到后端并关联到 OpenTelemetry span 中。未传入时使用 `"unknown"` 占位。

### 5.2 投递保障配置

每条路由可独立配置投递保障级别（`delivery_guarantee` 字段），共三种语义：

#### at_most_once（最多一次，默认）

- **行为:** 发即忘，不记录投递状态
- **适用场景:** 对丢失不敏感的查询类请求、幂等的只读操作
- **性能:** 最高，无额外的数据库写入
- **请求头:** 无特殊要求

```bash
# at_most_once 模式下直接调用即可
curl -X POST http://localhost:8080/gw/calculator/add \
  -H "Content-Type: application/json" \
  -d '{"a": 5, "b": 3}'
```

#### at_least_once（至少一次）

- **行为:** 记录投递状态，失败时自动重试
- **适用场景:** 数据写入类操作，允许重复投递
- **重试策略:** 首次失败后 1 秒重试，由 Retry Worker 按指数退避处理后续重试
- **死信处理:** 超出最大重试次数后进入死信队列，等待人工干预
- **请求头:** 无特殊要求

#### exactly_once（恰好一次）

- **行为:** 在 at_least_once 基础上增加幂等键去重
- **适用场景:** 金融交易、订单创建等不允许重复处理的操作
- **请求头:** 必须携带 `Idempotency-Key`

```bash
curl -X POST http://localhost:8080/gw/payment/create \
  -H "Content-Type: application/json" \
  -H "Idempotency-Key: order-12345-attempt-1" \
  -d '{"amount": 100, "currency": "CNY"}'
```

若同一个 `Idempotency-Key` 重复请求，网关会直接返回：

```json
{"status": "already_delivered"}
```

### 5.3 沙箱调用

沙箱请求通过 `/sandbox/*` 前缀发送，与 `/gw/*` 共享相同的路由表，但请求不转发到真实后端（Mock/Replay 模式），或经过额外的隔离控制（Proxy 模式）。

#### X-Sandbox-Mode 请求头

| 值 | 说明 |
|----|------|
| `mock` | 返回模拟数据（默认，可省略该头） |
| `replay` | 回放录制的请求-响应对 |
| `proxy` | 代理转发到真实后端 |

缺失该头时默认为 `mock` 模式。传入无效值会返回 `400 Bad Request`。

#### X-Sandbox-Session 请求头

值为沙箱会话 ID（UUID 格式）。

| 模式 | 是否必须 | 说明 |
|------|---------|------|
| mock | 可选 | 传入时读取会话的 `fixed_response` 配置 |
| replay | 必须 | 用于定位录制集 |
| proxy | 必须 | 用于读取代理配置和租户隔离信息 |

#### 三种模式的切换和使用

```bash
# Mock 模式（最简使用，无需会话）
curl -X POST http://localhost:8080/sandbox/calculator/add \
  -H "Content-Type: application/json" \
  -d '{"a": 1, "b": 2}'

# Mock 模式（带会话，获取固定响应）
curl -X POST http://localhost:8080/sandbox/calculator/add \
  -H "Content-Type: application/json" \
  -H "X-Sandbox-Mode: mock" \
  -H "X-Sandbox-Session: 550e8400-e29b-41d4-a716-446655440000" \
  -d '{"a": 1, "b": 2}'

# Replay 模式
curl -X POST http://localhost:8080/sandbox/calculator/add \
  -H "Content-Type: application/json" \
  -H "X-Sandbox-Mode: replay" \
  -H "X-Sandbox-Session: 550e8400-e29b-41d4-a716-446655440000" \
  -d '{"a": 1, "b": 2}'

# Proxy 模式
curl -X POST http://localhost:8080/sandbox/calculator/add \
  -H "Content-Type: application/json" \
  -H "X-Sandbox-Mode: proxy" \
  -H "X-Sandbox-Session: 550e8400-e29b-41d4-a716-446655440000" \
  -d '{"a": 1, "b": 2}'
```

### 5.4 Webhook 推送

当系统发生特定事件（如死信产生、合约生成完成等）时，会向已注册的 Webhook 订阅地址发送 HTTP POST 推送通知。

#### 配置下游接收推送

1. 在你的服务中暴露一个 HTTP POST 端点用于接收推送
2. 通过 API 或 Web 管理平台创建 Webhook 订阅，指定该端点 URL 和关注的事件类型
3. 系统在事件触发时会向该 URL 发送 JSON 格式的推送消息

推送请求格式：

```bash
POST <your-webhook-url>
Content-Type: application/json

{
  "event_type": "DeadLetter",
  "timestamp": "2026-03-22T10:30:00Z",
  "payload": { ... }
}
```

若推送失败（目标不可达或返回非 2xx），系统会记录投递失败事件。建议下游端点返回 `200 OK` 确认接收。

### 5.5 告警配置

平台支持通过环境变量配置告警通知推送，当系统产生告警事件时自动发送到指定的 Webhook 地址。

#### 环境变量

| 环境变量 | 说明 | 示例 |
|---------|------|------|
| `ALERT_WEBHOOK_URL` | 告警推送目标地址 | `https://hooks.slack.com/services/T.../B.../xxx` |
| `ALERT_WEBHOOK_TYPE` | 推送格式类型 | `slack` 或 `dingtalk` |

#### 配置示例

```bash
# Slack 告警
ALERT_WEBHOOK_URL=https://hooks.slack.com/services/T00000/B00000/xxxxx
ALERT_WEBHOOK_TYPE=slack

# 钉钉告警
ALERT_WEBHOOK_URL=https://oapi.dingtalk.com/robot/send?access_token=xxxxx
ALERT_WEBHOOK_TYPE=dingtalk
```

系统会根据 `ALERT_WEBHOOK_TYPE` 自动适配推送消息格式（Slack Block Kit 或钉钉 Markdown），无需额外配置。

---

## 6. 监控与可观测性

API-Anything 使用 OpenTelemetry 标准采集遥测数据（Traces、Metrics、Logs），通过 OTel Collector 分发到各后端存储。

### 基础设施组件

使用 `docker/docker-compose.yml` 一键启动所有可观测性组件：

```bash
cd docker
docker compose up -d
```

### Grafana 面板访问

- **地址:** http://localhost:3000
- **默认账号:** admin / admin
- **匿名访问:** 已启用（Viewer 角色）

Grafana 预配置了三个数据源：

| 数据源 | 类型 | 地址 | 用途 |
|--------|------|------|------|
| Prometheus | 默认 | http://prometheus:9090 | 指标查询 |
| Tempo | Traces | http://tempo:3200 | 链路追踪 |
| Loki | Logs | http://loki:3100 | 日志查询 |

数据源之间已配置关联：
- Tempo → Loki：通过 `trace_id` 从链路跳转到日志
- Tempo → Prometheus：从链路跳转到指标
- Loki → Tempo：通过正则匹配 `trace_id=(\w+)` 从日志跳转到链路

### Prometheus 指标查看

- **地址:** http://localhost:9090
- **采集间隔:** 15 秒

Prometheus 配置了两个采集目标：

| Job 名称 | 目标 | 说明 |
|----------|------|------|
| `otel-collector` | `otel-collector:8888` | OTel Collector 自身指标 |
| `api-anything` | `host.docker.internal:8080` | Platform API 应用指标 |

### Tempo 链路追踪

- **地址:** http://localhost:3200 (API)
- **推荐通过 Grafana 的 Explore 页面查询**

Platform API 在启动时初始化 OpenTelemetry tracing：
- 使用 gRPC 协议将 span 发送到 OTel Collector（`localhost:4317`）
- OTel Collector 将 traces 转发到 Tempo 存储
- 若 OTel Collector 不可用，服务会自动降级为纯日志模式，不影响正常运行

在 Grafana 中查看链路：
1. 进入 Explore 页面
2. 选择 Tempo 数据源
3. 按 `trace_id` 或服务名搜索

### Loki 日志查询

- **地址:** http://localhost:3100
- **推荐通过 Grafana 的 Explore 页面查询**

Platform API 输出 JSON 格式的结构化日志，便于 Loki 解析和查询。

日志级别通过环境变量 `RUST_LOG` 控制：

```bash
# 推荐开发环境配置
RUST_LOG=api_anything=debug,tower_http=debug

# 推荐生产环境配置
RUST_LOG=api_anything=info,tower_http=info
```

在 Grafana 中查询日志：
1. 进入 Explore 页面
2. 选择 Loki 数据源
3. 使用 LogQL 查询，例如 `{job="api-anything"} |= "error"`

### OTel Collector 数据流

OTel Collector 监听以下端口：

| 端口 | 协议 | 说明 |
|------|------|------|
| 4317 | gRPC | OTLP gRPC 接收端（Platform API 发送到此） |
| 4318 | HTTP | OTLP HTTP 接收端 |

数据流转路径：

```
Platform API → OTel Collector → ┬─ Traces  → Tempo
                                 ├─ Metrics → Prometheus (remote write)
                                 └─ Logs    → Loki
```

---

## 7. LLM 增强功能

API-Anything 支持使用 LLM（大语言模型）优化 WSDL 到 REST 的映射结果，使生成的 API 更符合 RESTful 设计最佳实践。

### 环境变量配置

LLM 功能为可选项，通过环境变量配置 API Key 即可启用。支持两种 LLM 提供商：

| 环境变量 | 提供商 | 默认模型 |
|---------|--------|---------|
| `ANTHROPIC_API_KEY` | Anthropic (Claude) | `claude-sonnet-4-20250514` |
| `OPENAI_API_KEY` | OpenAI | `gpt-4o` |

在 `.env` 文件中配置（参考 `.env.example`）：

```bash
# 二选一即可
ANTHROPIC_API_KEY=sk-ant-api03-...
# 或
OPENAI_API_KEY=sk-proj-...
```

### LLM 增强映射的效果

以 `calculator.wsdl` 为例，对比确定性映射和 LLM 增强映射的结果：

**确定性映射（默认）：**

| SOAP 操作 | HTTP 方法 | 路径 |
|-----------|----------|------|
| Add | POST | `/calculator/add` |
| GetHistory | POST | `/calculator/get-history` |

**LLM 增强映射：**

| SOAP 操作 | HTTP 方法 | 路径 |
|-----------|----------|------|
| Add | POST | `/api/v1/calculator/additions` |
| GetHistory | GET | `/api/v1/calculator/history` |

LLM 优化的具体表现：
- **HTTP 方法语义化:** `GetHistory` 从 `POST` 优化为 `GET`（读操作应使用 GET）
- **路径设计:** 采用资源化命名（`additions` 而非 `add`）和标准 API 版本前缀（`/api/v1/`）

### 降级机制说明

LLM 增强映射内置了完整的降级保护：

1. **LLM 不可用时（网络错误、API Key 无效、服务限流等）：** 自动降级到确定性映射结果，记录 `WARN` 级别日志，生成流程不中断
2. **LLM 返回无效响应时（无法解析的 JSON、格式不匹配等）：** 同样降级到确定性映射
3. **部分操作匹配时：** LLM 建议中能匹配到的操作应用优化，未匹配的操作保留确定性映射结果

降级设计原则：**LLM 故障不应阻断整个生成流程**。即使 LLM 完全不可用，平台仍能正常工作。

### JSON 提取

LLM 返回的文本可能包含 Markdown code fence 包裹的 JSON。系统会按以下优先级自动提取：

1. ` ```json ... ``` ` — 最明确的 JSON code fence
2. ` ``` ... ``` ` — 通用 code fence（内容以 `{` 或 `[` 开头）
3. 裸 JSON — 纯 JSON 文本

---

## 8. 常见问题 (FAQ)

### Q1: 启动 Platform API 时提示数据库连接失败？

确认 PostgreSQL 已启动且连接参数正确：

```bash
# 启动基础设施
cd docker && docker compose up -d postgres

# 检查 PostgreSQL 状态
docker compose ps postgres

# 确认连接参数（默认值）
# DATABASE_URL=postgres://api_anything:api_anything@localhost:5432/api_anything
```

Platform API 启动时会自动执行数据库迁移（`run_migrations`），无需手动初始化表结构。

### Q2: 执行 `generate` 命令时提示 "Failed to read WSDL file"？

检查文件路径是否正确。命令使用相对路径时，相对的是当前工作目录。建议使用绝对路径：

```bash
api-anything generate \
  --source /full/path/to/service.wsdl \
  --project my-service
```

### Q3: 网关返回 404 但路由已生成？

路由在 CLI 生成后写入数据库，但 Platform API 的 DynamicRouter 在启动时加载。若 API 服务在路由生成之前启动，需要重启 Platform API 使路由表重新加载：

```bash
# 1. 先通过 CLI 生成路由
api-anything generate --source service.wsdl --project my-service

# 2. 再启动或重启 Platform API
cargo run --bin api-anything-platform-api
```

### Q4: 如何修改 API 监听端口？

通过环境变量配置：

```bash
API_HOST=0.0.0.0  # 监听地址，默认 0.0.0.0
API_PORT=9090     # 监听端口，默认 8080
```

所有配置项及默认值：

| 环境变量 | 默认值 | 说明 |
|---------|--------|------|
| `DATABASE_URL` | `postgres://api_anything:api_anything@localhost:5432/api_anything` | PostgreSQL 连接串 |
| `KAFKA_BROKERS` | `localhost:9092` | Kafka broker 地址 |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | `http://localhost:4317` | OTel Collector gRPC 端点 |
| `API_HOST` | `0.0.0.0` | API 监听地址 |
| `API_PORT` | `8080` | API 监听端口 |
| `RUST_LOG` | `info` | 日志级别 |

### Q5: OTel Collector 不可用会影响服务吗？

不会。Platform API 在初始化 tracing 时，若 OTel Collector 连接失败，会自动降级为纯 `tracing-subscriber` 日志输出。服务正常运行，但链路追踪数据将不可用。stderr 中会输出警告信息：

```
WARN: OTel tracing init failed, falling back to tracing-subscriber only: ...
```

### Q6: 沙箱 Mock 模式返回的数据格式不符合预期？

Mock 模式根据路由的 `response_schema` 自动生成数据。如需固定返回内容：

1. 创建带 `fixed_response` 配置的沙箱会话：

```bash
curl -X POST http://localhost:8080/api/v1/projects/{project-id}/sandbox-sessions \
  -H "Content-Type: application/json" \
  -d '{
    "tenant_id": "dev-team",
    "mode": "mock",
    "config": {
      "fixed_response": {"result": 42, "status": "ok"}
    },
    "expires_in_hours": 24
  }'
```

2. 请求时带上返回的 session ID。

### Q7: 死信队列中的记录如何产生？

当路由配置了 `at_least_once` 或 `exactly_once` 投递保障时：

1. 网关投递到后端失败 → 记录状态变为 `Failed`，设置 1 秒后重试
2. Retry Worker 按指数退避重试
3. 超出最大重试次数后 → 状态变为 `Dead`，进入死信队列

运维人员可在死信队列中：
- **Retry:** 重新尝试投递
- **Resolve:** 标记为已通过其他渠道解决

### Q8: `Idempotency-Key` 有什么格式要求？

`Idempotency-Key` 为任意字符串，只要在同一路由内唯一即可。建议包含业务语义，便于排查：

```
Idempotency-Key: order-12345-create-v1
Idempotency-Key: payment-67890-20260322T103000
```

### Q9: 如何查看单条投递记录的详细信息？

```bash
curl http://localhost:8080/api/v1/compensation/delivery-records/{record-id}
```

返回完整的投递记录，包括原始请求体、响应体、错误信息、重试次数、下次重试时间等。

### Q10: 支持哪些来源类型？

当前支持的来源类型及其 CLI 对应关系：

| 来源类型 | CLI 命令 | 状态 |
|---------|---------|------|
| `wsdl` | `api-anything generate` | 可用 |
| `cli` | `api-anything generate-cli` | 可用 |
| `ssh` | `api-anything generate-ssh` | 可用 |
| `pty` | — | 规划中 |
| `odata` | — | 规划中 |

### Q11: 如何开发自定义协议插件？

API-Anything 支持通过动态库实现自定义协议适配器插件。开发步骤：

1. 使用 `plugin-sdk` crate 作为开发依赖，实现 `ProtocolPlugin` trait
2. 编译为动态库（Linux 下为 `.so`，macOS 下为 `.dylib`）
3. 将编译产物放入插件目录（由 `PLUGIN_DIR` 环境变量指定，默认为 `./plugins`）
4. 通过 API 触发插件扫描，或重启服务自动加载

```bash
# 编译插件
cargo build --release -p my-custom-plugin
cp target/release/libmy_custom_plugin.so ./plugins/

# 触发扫描
curl -X POST http://localhost:8080/api/v1/plugins/scan

# 查看已加载插件
curl http://localhost:8080/api/v1/plugins
```

### Q12: 如何配置 Kafka 事件总线？

平台默认使用 PostgreSQL 作为事件总线（`EVENT_BUS_TYPE=pg`）。若需使用 Kafka 作为事件总线以获得更高吞吐量，配置以下环境变量：

```bash
EVENT_BUS_TYPE=kafka
KAFKA_BROKERS=localhost:9092
```

PG 模式下事件直接写入 PostgreSQL 表并通过轮询消费，适用于中小规模部署。Kafka 模式适用于高吞吐、多消费者场景。

---

## 9. 安全配置

### 9.1 TLS/HTTPS 配置

**何时需要**：生产环境必须开启，确保传输层加密。

**如何配置**：

```bash
# 生成自签名证书（开发测试用）
openssl req -x509 -newkey rsa:4096 -keyout key.pem -out cert.pem -days 365 -nodes

# .env 配置
TLS_CERT_PATH=./cert.pem
TLS_KEY_PATH=./key.pem
```

- 两个路径**同时配置**时启用 HTTPS，启动日志显示 `Listening on https://...`
- 不配置时自动使用 HTTP（开发模式），启动日志显示 `Listening on http://...`
- TLS 实现基于 `axum-server` + `rustls`，无需 OpenSSL 运行时依赖

> **注意**：只配置其中一个路径（cert 或 key）不会生效，系统仍以 HTTP 模式启动。

### 9.2 JWT 认证配置

**何时需要**：对外暴露 API 时必须开启，阻止未授权访问。

**如何配置**：

```bash
AUTH_ENABLED=true
JWT_SECRET=your-production-secret-at-least-256-bits
```

**白名单路径**（无需 token 即可访问）：
- `/health` — 存活探针（K8s liveness probe）
- `/health/ready` — 就绪探针（K8s readiness probe）
- `/api/v1/docs` — Swagger UI 文档页面

**如何获取 token**：由外部身份提供方（如 Keycloak、Auth0、自建 IdP）签发 HS256 JWT。

**请求示例**：

```bash
curl -H "Authorization: Bearer eyJhbG..." http://localhost:8080/gw/api/v1/orders
```

**Claims 结构**：

```json
{
  "sub": "user-id",
  "role": "admin",
  "exp": 1234567890
}
```

- `sub`（必填）：用户唯一标识
- `role`（可选）：用户角色，注入到请求 extensions 供下游 handler 消费
- `exp`（必填）：过期时间戳（Unix epoch 秒）

**未认证请求响应**：返回 `401 Unauthorized`。

> **开发模式**：`AUTH_ENABLED` 默认为 `false`，开发环境无需配置 JWT 即可正常使用所有 API。

### 9.3 敏感数据加密

**何时需要**：存储 SSH 密码、SOAP 凭证等敏感信息时，避免数据库中明文暴露。

**如何配置**：

```bash
# 生成 256-bit 密钥（64 个 hex 字符）
openssl rand -hex 32
# 输出类似：a1b2c3d4e5f6...

ENCRYPTION_KEY=a1b2c3d4e5f6...
```

- **加密算法**：AES-256-GCM（通过 `ring` 库实现）
- **加密范围**：`BackendBinding.endpoint_config` 中的敏感字段
- **不配置时**：明文存储（向后兼容，开发环境零配置可用）
- **随机 nonce**：每次加密使用随机 12 字节 nonce，相同明文产生不同密文

> **警告**：密钥丢失将**无法解密**已加密的数据，请妥善备份 `ENCRYPTION_KEY`。

---

## 10. 运维配置

### 10.1 路由热加载

平台内置路由轮询机制，无需重启即可感知路由变更。

- **默认间隔**：每 5 秒检查数据库中的路由变更
- **配置轮询间隔**：`ROUTE_POLL_INTERVAL_SECS=5`（单位：秒）
- **工作原理**：比较路由数量 → 有变化时通过 `RouteLoader` 原子替换路由表 → 零停机
- **使用场景**：通过 CLI `generate` / `generate-ssh` / `generate-cli` 生成新路由后，网关自动感知并加载，无需重启服务

**日志示例**：
```
INFO Routes hot-reloaded routes=12
```

### 10.2 WebSocket 实时推送

平台提供 WebSocket 端点用于实时事件推送，Web 面板通过此连接展示路由变更、死信告警等实时通知。

- **端点**：`ws://localhost:8080/ws`（开启 TLS 时为 `wss://localhost:8080/ws`）
- **推送机制**：服务端每 2 秒轮询 `events` 表，推送新事件给所有连接的客户端
- **事件类型**：RouteUpdated、DeliveryFailed、DeadLetter、GenerationCompleted 等
- **前端自动连接**：通过 `useWebSocket` hook 自动建立连接，断线 3 秒后自动重连
- **多副本安全**：基于数据库轮询而非内存广播，多实例部署时事件不丢失

**事件消息格式**：

```json
{
  "id": "uuid",
  "type": "RouteUpdated",
  "payload": { ... },
  "timestamp": "2026-03-23T10:00:00Z"
}
```

**命令行测试**：

```bash
# 使用 wscat 连接
wscat -c ws://localhost:8080/ws
```

### 10.3 OTel 自定义指标

网关自动暴露以下 Prometheus 指标，通过 OpenTelemetry Collector 推送到 Prometheus，Grafana Dashboard 自动展示。

| 指标名 | 类型 | 说明 |
|--------|------|------|
| `gateway_request_total` | Counter | 网关请求总量（按 route/method/status 分组） |
| `gateway_request_duration_seconds` | Histogram | 网关请求端到端延迟分布（含路由匹配 + 后端调用 + 响应序列化） |
| `backend_execute_duration_seconds` | Histogram | 后端协议调用延迟分布（仅 adapter.execute 部分，按 route/protocol 分组） |
| `delivery_retry_total` | Counter | 投递重试次数（监控重试风暴） |
| `delivery_dead_letter_total` | Counter | 死信数量（超过阈值应触发告警） |

**OTel Collector 配置**：通过 `OTEL_EXPORTER_OTLP_ENDPOINT`（默认 `http://localhost:4317`）将指标和链路数据推送到 Collector。

---

## API 端点速查表

### 管理 API (`/api/v1/`)

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/health` | 存活探针 |
| GET | `/health/ready` | 就绪探针（含数据库连通性检查） |
| POST | `/api/v1/projects` | 创建项目 |
| GET | `/api/v1/projects` | 列出所有项目 |
| GET | `/api/v1/projects/{id}` | 获取项目详情 |
| DELETE | `/api/v1/projects/{id}` | 删除项目 |
| POST | `/api/v1/projects/{project_id}/sandbox-sessions` | 创建沙箱会话 |
| GET | `/api/v1/projects/{project_id}/sandbox-sessions` | 列出项目的沙箱会话 |
| DELETE | `/api/v1/sandbox-sessions/{id}` | 删除沙箱会话 |
| GET | `/api/v1/compensation/dead-letters` | 列出死信队列 |
| POST | `/api/v1/compensation/dead-letters/{id}/retry` | 重试单条死信 |
| POST | `/api/v1/compensation/dead-letters/{id}/resolve` | 标记死信已解决 |
| POST | `/api/v1/compensation/dead-letters/batch-retry` | 批量重试死信 |
| GET | `/api/v1/compensation/delivery-records/{id}` | 查看投递记录详情 |
| GET | `/api/v1/docs` | Swagger UI 页面 |
| GET | `/api/v1/docs/openapi.json` | OpenAPI 3.0 规范 (JSON) |
| GET | `/api/v1/docs/agent-prompt` | AI Agent 提示词 (Markdown) |
| GET | `/api/v1/docs/sdk/{language}` | 生成指定语言的 SDK 代码 |
| POST | `/api/v1/webhooks` | 创建 Webhook 订阅 |
| GET | `/api/v1/webhooks` | 列出所有 Webhook 订阅 |
| DELETE | `/api/v1/webhooks/{id}` | 删除 Webhook 订阅 |
| GET | `/api/v1/plugins` | 列出已加载的插件 |
| POST | `/api/v1/plugins/scan` | 扫描并加载插件目录 |
| GET | `/api/v1/sandbox-sessions/{id}/recordings` | 查看沙箱会话录制 |
| DELETE | `/api/v1/sandbox-sessions/{id}/recordings` | 清空沙箱会话录制 |

### 网关 (`/gw/`)

| 方法 | 路径 | 说明 |
|------|------|------|
| ANY | `/gw/{*rest}` | 动态路由匹配，转发到后端 |

### 沙箱 (`/sandbox/`)

| 方法 | 路径 | 说明 |
|------|------|------|
| ANY | `/sandbox/{*rest}` | 沙箱路由匹配，支持 mock/replay/proxy |

### 死信查询参数

`GET /api/v1/compensation/dead-letters` 支持以下查询参数：

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `route_id` | UUID | 无 | 按路由过滤，不传则返回全局视图 |
| `limit` | 整数 | 50 | 每页条数 |
| `offset` | 整数 | 0 | 偏移量 |
