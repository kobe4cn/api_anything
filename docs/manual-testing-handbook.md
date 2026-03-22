# API-Anything 手工测试手册

> 版本：1.0
> 最后更新：2026-03-22
> 适用对象：QA 测试人员

---

## 目录

1. [测试环境准备](#1-测试环境准备)
2. [测试数据准备](#2-测试数据准备)
3. [API 后端测试用例](#3-api-后端测试用例)
4. [沙箱测试用例](#4-沙箱测试用例)
5. [补偿机制测试用例](#5-补偿机制测试用例)
6. [前端 Web 测试用例](#6-前端-web-测试用例)
7. [前端测试截图说明](#7-前端测试截图说明)
8. [Phase 6 功能测试用例](#8-phase-6-功能测试用例)

---

## 1. 测试环境准备

### 1.1 服务启动检查清单

在开始测试前，确认以下服务全部就绪：

| 序号 | 检查项 | 检查方法 | 预期结果 |
|------|--------|---------|---------|
| 1 | PostgreSQL 数据库 | `psql -h localhost -p 5432 -U postgres -c "SELECT 1"` | 返回 1，无连接错误 |
| 2 | 平台 API 服务 | `curl http://localhost:8080/health` | `{"status":"ok"}` |
| 3 | 数据库就绪探针 | `curl http://localhost:8080/health/ready` | `{"status":"ready","db":"connected"}` |
| 4 | 前端静态资源 | 浏览器访问 `http://localhost:8080/` | 显示 Dashboard 页面 |
| 5 | 数据库迁移 | 检查 `sqlx migrate run` 是否已执行 | 所有迁移脚本已应用 |

### 1.2 测试数据准备脚本

将以下内容保存为 `setup-test-env.sh`，在测试开始前执行：

```bash
#!/bin/bash
# 测试环境初始化脚本
BASE_URL="http://localhost:8080"

echo "=== 创建 WSDL 测试项目 ==="
WSDL_PROJECT=$(curl -s -X POST "$BASE_URL/api/v1/projects" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "order-soap-service",
    "description": "遗留 SOAP 订单服务",
    "owner": "team-platform",
    "source_type": "wsdl"
  }')
echo "$WSDL_PROJECT"
WSDL_PROJECT_ID=$(echo "$WSDL_PROJECT" | jq -r '.id')

echo "=== 创建 CLI 测试项目 ==="
CLI_PROJECT=$(curl -s -X POST "$BASE_URL/api/v1/projects" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "db-management-tool",
    "description": "数据库管理 CLI 工具",
    "owner": "team-dba",
    "source_type": "cli"
  }')
echo "$CLI_PROJECT"
CLI_PROJECT_ID=$(echo "$CLI_PROJECT" | jq -r '.id')

echo "=== 创建 SSH 测试项目 ==="
SSH_PROJECT=$(curl -s -X POST "$BASE_URL/api/v1/projects" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "network-switch-mgmt",
    "description": "网络交换机管理",
    "owner": "team-network",
    "source_type": "ssh"
  }')
echo "$SSH_PROJECT"
SSH_PROJECT_ID=$(echo "$SSH_PROJECT" | jq -r '.id')

echo ""
echo "=== 项目 ID 汇总 ==="
echo "WSDL 项目: $WSDL_PROJECT_ID"
echo "CLI  项目: $CLI_PROJECT_ID"
echo "SSH  项目: $SSH_PROJECT_ID"
```

### 1.3 浏览器和工具要求

| 工具 | 最低版本 | 用途 |
|------|---------|------|
| Chrome / Firefox / Edge | 最新稳定版 | 前端 UI 测试 |
| curl | 7.68+ | API 接口测试 |
| jq | 1.6+ | JSON 响应解析 |
| psql | 14+ | 数据库直查验证 |
| hey / ab | 最新版 | 压力/限流测试 |

> **注意**：所有 curl 命令默认以 `http://localhost:8080` 为 BASE_URL。实际测试时根据部署环境替换。

---

## 2. 测试数据准备

### 2.1 WSDL 测试数据

测试数据文件位于 `docs/test-data/` 目录下。

#### 简单 WSDL — calculator.wsdl（已有）

- **路径**: `crates/generator/tests/fixtures/calculator.wsdl`
- **操作**: Add（两数相加）、GetHistory（查询历史记录）
- **用途**: 基础 WSDL 解析与 SOAP 代理验证

#### 复杂 WSDL — complex-order-service.wsdl

- **路径**: `docs/test-data/complex-order-service.wsdl`
- **目标服务**: 模拟订单系统（`http://legacy-erp.internal:8080/soap/orders`）
- **包含操作**:

| 操作 | 说明 | 关键测试点 |
|------|------|-----------|
| GetOrder | 按 ID 查询订单 | 路径参数、嵌套响应（Address、OrderItem 数组） |
| CreateOrder | 创建订单 | 复杂嵌套输入（多层对象、数组字段、可选字段） |
| ListOrders | 分页查询订单列表 | 分页参数（PaginationRequest）、过滤条件 |
| CancelOrder | 取消订单 | 状态变更、布尔参数 |
| GetOrderHistory | 查询订单操作历史 | 数组返回（OrderHistoryEntry）、分页 |

#### 错误场景 WSDL

在测试中使用以下非法输入来验证错误处理：

- 空文件（0 字节）
- 非 XML 文本文件（如纯文本 "hello world"）
- 合法 XML 但非 WSDL 格式（如普通 HTML）
- 缺少 `<portType>` 的不完整 WSDL

### 2.2 CLI 测试数据

#### 主命令帮助 — complex-cli-help.txt

- **路径**: `docs/test-data/complex-cli-help.txt`
- **模拟工具**: `dbctl 3.5.2` — 数据库管理工具
- **子命令**: connect、query、backup、restore、migrate、status、users、tables、export、import
- **全局选项**: --host、--port、--user、--database、--format 等

#### 子命令帮助 — complex-cli-subcommand-help.txt

- **路径**: `docs/test-data/complex-cli-subcommand-help.txt`
- **模拟子命令**: `dbctl query`
- **包含**: 位置参数（SQL）、多种选项（--file、--params、--limit）、使用示例

#### 模拟脚本 — mock-db-tool.sh

- **路径**: `docs/test-data/mock-db-tool.sh`
- **可执行**: 已设置 `chmod +x`
- **支持子命令**:

| 子命令 | 输出格式 | 用途 |
|--------|---------|------|
| `query <SQL>` | JSON（默认）或 table | 测试 JSON 输出解析和文本输出解析 |
| `status` | JSON | 测试嵌套 JSON 输出 |
| `backup --database <name>` | JSON | 测试带必需参数的命令 |
| `users list` | JSON 数组 | 测试二级子命令 |
| `users create` | JSON | 测试创建操作 |
| 未知命令 | stderr + exit 1 | 测试错误处理 |

**使用示例**:

```bash
# JSON 输出
./docs/test-data/mock-db-tool.sh query "SELECT * FROM users" --format json

# 表格输出
./docs/test-data/mock-db-tool.sh query "SELECT * FROM users" --format table

# 数据库状态
./docs/test-data/mock-db-tool.sh status

# 备份（需要 --database 参数）
./docs/test-data/mock-db-tool.sh backup --database mydb --output /tmp/backup.sql.gz

# 错误场景
./docs/test-data/mock-db-tool.sh unknown_command  # exit code 1
./docs/test-data/mock-db-tool.sh query            # 缺少 SQL，exit code 1
```

### 2.3 SSH 测试数据

#### 网络设备 — network-switch-ssh.txt

- **路径**: `docs/test-data/network-switch-ssh.txt`
- **模拟设备**: Cisco Catalyst 9300 交换机（`192.168.1.1`）
- **包含命令**:

| 命令 | 参数 | 输出格式 |
|------|------|---------|
| `show interfaces status` | 无 | table |
| `show vlan brief` | 无 | table |
| `show running-config interface {interface}` | interface（如 `Gi1/0/2`） | text |
| `show mac address-table vlan {vlan_id}` | vlan_id（如 `100`） | table |
| `show ip interface brief` | 无 | table |
| `show environment` | 无 | text |

#### 服务器运维 — server-management-ssh.txt

- **路径**: `docs/test-data/server-management-ssh.txt`
- **模拟设备**: Ubuntu 22.04 应用服务器（`10.0.2.100`）
- **包含命令**:

| 命令 | 参数 | 输出格式 |
|------|------|---------|
| `systemctl status {service_name}` | service_name | text |
| `df -h` | 无 | table |
| `free -h` | 无 | table |
| `docker ps --format ...` | 无 | table |
| `journalctl -u {service_name} --since "{since_time}" -n {lines}` | service_name、since_time、lines | text |
| `ss -tlnp` | 无 | table |
| `top -bn1 \| head -20` | 无 | text |

---

## 3. API 后端测试用例

> **错误响应格式说明**: 所有错误响应均符合 RFC 7807 (Problem Details)，Content-Type 为 `application/problem+json`，包含 `type`、`title`、`status`、`detail` 字段。

### 3.1 项目管理 API

| 编号 | 用例名称 | 请求方法/路径 | 请求体 / 参数 | 预期状态码 | 预期响应 | 验证要点 |
|------|---------|--------------|--------------|-----------|---------|---------|
| PM-001 | 创建 WSDL 项目 | `POST /api/v1/projects` | `{"name":"calc-service","description":"计算器服务","owner":"team-a","source_type":"wsdl"}` | 201 | 返回完整 Project 对象，含 `id`、`created_at` | id 为合法 UUID；source_type 为 `wsdl` |
| PM-002 | 创建 CLI 项目 | `POST /api/v1/projects` | `{"name":"db-tool","description":"数据库工具","owner":"team-dba","source_type":"cli"}` | 201 | 返回 Project，source_type 为 `cli` | source_type 序列化为小写 snake_case |
| PM-003 | 创建 SSH 项目 | `POST /api/v1/projects` | `{"name":"switch-mgmt","description":"交换机管理","owner":"team-net","source_type":"ssh"}` | 201 | 返回 Project，source_type 为 `ssh` | - |
| PM-004 | 创建 PTY 项目 | `POST /api/v1/projects` | `{"name":"pty-tool","description":"PTY 工具","owner":"team-x","source_type":"pty"}` | 201 | 返回 Project，source_type 为 `pty` | 验证 pty 类型被系统接受 |
| PM-005 | 缺少必填字段 | `POST /api/v1/projects` | `{"name":"test"}` — 缺少 description、owner、source_type | 400 / 422 | RFC 7807 错误响应 | 检查 detail 字段包含缺失字段信息 |
| PM-006 | 无效 source_type | `POST /api/v1/projects` | `{"name":"x","description":"x","owner":"x","source_type":"invalid"}` | 400 / 422 | 错误响应 | 验证不接受非法枚举值 |
| PM-007 | 获取项目详情 | `GET /api/v1/projects/{id}` | PM-001 返回的 id | 200 | 返回与创建时一致的 Project | 所有字段值一致 |
| PM-008 | 获取不存在项目 | `GET /api/v1/projects/{random-uuid}` | 随机 UUID | 404 | `{"type":"about:blank","title":"Not Found","status":404,"detail":"..."}` | Content-Type 为 `application/problem+json` |
| PM-009 | 无效 UUID 格式 | `GET /api/v1/projects/not-a-uuid` | 字符串 `not-a-uuid` | 400 | 错误响应 | 验证路径参数校验 |
| PM-010 | 列出所有项目 | `GET /api/v1/projects` | 无 | 200 | JSON 数组，包含之前创建的所有项目 | 数组长度 >= 创建的数量 |
| PM-011 | 空列表 | `GET /api/v1/projects` | 前提：数据库中无项目 | 200 | 空数组 `[]` | 不是 404，而是空数组 |
| PM-012 | 删除项目 | `DELETE /api/v1/projects/{id}` | PM-001 返回的 id | 204 | 无响应体 | 再次 GET 该 id 应返回 404 |
| PM-013 | 删除不存在的项目 | `DELETE /api/v1/projects/{random-uuid}` | 随机 UUID | 404 | RFC 7807 错误 | title 为 "Not Found" |

**cURL 示例**:

```bash
# PM-001: 创建 WSDL 项目
curl -v -X POST http://localhost:8080/api/v1/projects \
  -H "Content-Type: application/json" \
  -d '{"name":"calc-service","description":"计算器服务","owner":"team-a","source_type":"wsdl"}'

# PM-007: 获取项目详情（替换 {id}）
curl -s http://localhost:8080/api/v1/projects/{id} | jq .

# PM-010: 列出所有项目
curl -s http://localhost:8080/api/v1/projects | jq .

# PM-012: 删除项目
curl -v -X DELETE http://localhost:8080/api/v1/projects/{id}
```

### 3.2 生成流水线测试

> 生成流水线将源定义（WSDL/CLI 帮助/SSH 交互样本）解析为路由表和后端绑定，写入数据库。以下用例假设生成器已集成到平台或通过 CLI 工具单独触发。

| 编号 | 用例名称 | 输入源 | 操作 | 预期结果 | 验证要点 |
|------|---------|--------|------|---------|---------|
| GEN-001 | WSDL 生成（简单） | `calculator.wsdl` | 对 WSDL 项目触发生成 | 成功，生成 2 条路由（Add、GetHistory） | 路由 method/path 正确；request_schema 和 response_schema 字段非空 |
| GEN-002 | WSDL 生成（复杂） | `complex-order-service.wsdl` | 对 WSDL 项目触发生成 | 成功，生成 5 条路由 | GetOrder、CreateOrder、ListOrders、CancelOrder、GetOrderHistory 全部存在 |
| GEN-003 | CLI 主命令生成 | `complex-cli-help.txt` | 对 CLI 项目触发生成 | 成功，为每个子命令生成路由 | 子命令 query、status、backup 等映射为 API 路由 |
| GEN-004 | CLI 子命令详情 | `complex-cli-help.txt` + `complex-cli-subcommand-help.txt` | 带子命令帮助的生成 | 路由包含子命令参数 schema | query 路由的 request_schema 包含 sql、format、limit 等字段 |
| GEN-005 | SSH 生成 | `network-switch-ssh.txt` | 对 SSH 项目触发生成 | 成功，为每个 SSH 命令生成路由 | 带参数的命令（如 `show running-config interface {interface}`）在路由路径中体现 |
| GEN-006 | 无效 WSDL | 空文件 / 非 XML 文本 | 触发生成 | 失败或生成 0 条路由 | 不应 panic；应返回明确错误信息 |
| GEN-007 | 产物验证 | GEN-001 成功后 | 检查 OpenAPI JSON | `/api/v1/docs/openapi.json` 包含生成的路由 | paths 下有 `/gw/...` 前缀的路径 |
| GEN-008 | Agent Prompt 验证 | GEN-001 成功后 | `GET /api/v1/docs/agent-prompt` | Markdown 格式文本，包含路由信息 | 每个路由有 method、path、protocol、request/response schema |

### 3.3 网关代理测试 (SOAP)

> 前提条件：已通过 WSDL 生成了路由，路由表中存在对应的 SOAP 后端绑定。

| 编号 | 用例名称 | 请求 | 前提/模拟条件 | 预期状态码 | 预期响应 | 验证要点 |
|------|---------|------|-------------|-----------|---------|---------|
| GW-SOAP-001 | 正常 SOAP 代理 — Add | `POST /gw/api/v1/calculator/add` `{"a":3,"b":5}` | SOAP 后端正常运行 | 200 | `{"result":8}` | JSON 响应（非 SOAP XML）；body 与 SOAP 后端返回一致 |
| GW-SOAP-002 | 正常 SOAP 代理 — GetHistory | `POST /gw/api/v1/calculator/get-history` `{"limit":10}` | SOAP 后端正常 | 200 | JSON 数组或包含 entries 的对象 | 数组类型正确序列化 |
| GW-SOAP-003 | 复杂请求 — CreateOrder | `POST /gw/api/v1/order/create-order` 含嵌套 JSON | SOAP 后端正常 | 200 | 包含 order_id、status | 嵌套对象（Address）和数组（items）正确转为 SOAP XML 发送 |
| GW-SOAP-004 | 后端不可达 | `POST /gw/api/v1/calculator/add` `{"a":1,"b":2}` | 关闭 SOAP 后端服务 | 502 | `{"type":"about:blank","title":"Bad Gateway","status":502,"detail":"..."}` | Content-Type 为 `application/problem+json` |
| GW-SOAP-005 | 请求超时 | `POST /gw/api/v1/calculator/add` | 后端人为延迟超过 timeout_ms | 504 | `{"title":"Gateway Timeout","status":504,"detail":"Backend timeout after NNNms"}` | detail 中包含实际超时时长 |
| GW-SOAP-006 | SOAP Fault | `POST /gw/api/v1/calculator/add` | 后端返回 SOAP Fault XML | 502 | RFC 7807，status=502 | detail 中包含 SOAP Fault 信息 |
| GW-SOAP-007 | 路由不存在 | `GET /gw/api/v1/nonexistent/endpoint` | - | 404 | `{"title":"Not Found","status":404}` | detail 包含 "No route matches" |
| GW-SOAP-008 | 请求体过大 | `POST /gw/api/v1/calculator/add` body > 10MB | - | 400 | 错误响应 | 验证 10MB 请求体限制 |
| GW-SOAP-009 | 非 JSON 请求体 | `POST /gw/api/v1/calculator/add` body 为纯文本 | - | 200 或其他 | body 降级为 String 类型 Value | 不应返回 JSON 解析错误 |
| GW-SOAP-010 | traceparent 透传 | 请求头含 `traceparent: 00-abc...` | - | 200 | - | 检查后端收到的 trace_id 与请求头一致 |

**cURL 示例**:

```bash
# GW-SOAP-001: 正常代理
curl -v -X POST http://localhost:8080/gw/api/v1/calculator/add \
  -H "Content-Type: application/json" \
  -d '{"a":3,"b":5}'

# GW-SOAP-007: 不存在的路由
curl -v http://localhost:8080/gw/api/v1/nonexistent/endpoint

# GW-SOAP-010: 带 traceparent
curl -X POST http://localhost:8080/gw/api/v1/calculator/add \
  -H "Content-Type: application/json" \
  -H "traceparent: 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01" \
  -d '{"a":1,"b":2}'
```

### 3.4 网关代理测试 (CLI)

> 前提条件：已通过 CLI 帮助文本生成路由，后端绑定指向 CLI 可执行文件（如 mock-db-tool.sh）。

| 编号 | 用例名称 | 请求 | 前提/模拟条件 | 预期状态码 | 预期响应 | 验证要点 |
|------|---------|------|-------------|-----------|---------|---------|
| GW-CLI-001 | 正常命令执行 — query | `POST /gw/api/v1/dbctl/query` `{"sql":"SELECT * FROM users","format":"json"}` | mock-db-tool.sh 可执行 | 200 | JSON 对象含 columns、rows | 命令输出的 JSON 被正确解析后返回 |
| GW-CLI-002 | 正常命令执行 — status | `GET /gw/api/v1/dbctl/status` | mock-db-tool.sh | 200 | JSON 对象含 host、version、connections | 嵌套 JSON（connections 对象）保持结构 |
| GW-CLI-003 | 命令失败（exit code != 0） | `POST /gw/api/v1/dbctl/query` 缺少 sql 参数 | mock-db-tool.sh query 无参数 → exit 1 | 500 | 错误响应 | detail/stderr 包含 "SQL query is required" |
| GW-CLI-004 | 命令不存在 | 路由绑定指向 `/usr/bin/nonexistent_program` | 程序不存在 | 502 | `{"title":"Bad Gateway","status":502}` | 区分命令不存在（502）和命令执行失败（500） |
| GW-CLI-005 | JSON 输出解析 | `POST /gw/api/v1/dbctl/backup` `{"database":"mydb"}` | mock-db-tool.sh | 200 | `{"backup_id":"BK-...","database":"mydb","status":"completed"}` | JSON stdout 被解析为结构化响应 |
| GW-CLI-006 | 文本输出（非 JSON） | `POST /gw/api/v1/dbctl/query` `{"sql":"SELECT 1","format":"table"}` | mock-db-tool.sh | 200 | 文本表格被包装为 JSON | 验证 Regex 输出解析或 String 降级 |
| GW-CLI-007 | 二级子命令 | `GET /gw/api/v1/dbctl/users/list` | mock-db-tool.sh | 200 | JSON 数组 | 多级子命令路由正确映射 |

### 3.5 网关代理测试 (SSH)

> 前提条件：已通过 SSH 交互样本生成路由，后端绑定配置了 SSH 连接信息。

| 编号 | 用例名称 | 请求 | 前提/模拟条件 | 预期状态码 | 预期响应 | 验证要点 |
|------|---------|------|-------------|-----------|---------|---------|
| GW-SSH-001 | SSH 命令执行 — show interfaces | `GET /gw/api/v1/switch/show-interfaces-status` | SSH 主机可达 | 200 | 接口状态的结构化 JSON | table 格式输出被解析为 JSON |
| GW-SSH-002 | SSH 命令执行 — show vlan | `GET /gw/api/v1/switch/show-vlan-brief` | SSH 主机可达 | 200 | VLAN 配置的 JSON | - |
| GW-SSH-003 | 带参数的 SSH 命令 | `GET /gw/api/v1/switch/show-running-config-interface/{interface}` 参数 `Gi1/0/2` | SSH 主机可达 | 200 | 接口配置文本 | 路径参数正确传入 SSH 命令模板 |
| GW-SSH-004 | SSH 连接失败 | `GET /gw/api/v1/switch/show-interfaces-status` | SSH 主机不可达或凭证错误 | 502 | `{"title":"Bad Gateway","status":502}` | 不暴露 SSH 凭证等敏感信息 |
| GW-SSH-005 | 服务器运维 — systemctl | `GET /gw/api/v1/server/systemctl-status/{service_name}` 参数 `nginx` | SSH 主机可达 | 200 | 服务状态信息 | 多参数模板正确替换 |
| GW-SSH-006 | 服务器运维 — docker ps | `GET /gw/api/v1/server/docker-ps` | SSH 主机可达 | 200 | 容器列表 JSON | - |

### 3.6 保护层测试

> 保护层在 dispatcher 内部按 限流 → 熔断 → 超时 顺序执行，配置存储在 BackendBinding 的 `rate_limit_config`、`circuit_breaker_config`、`timeout_ms` 字段中。

| 编号 | 用例名称 | 操作步骤 | 预期状态码 | 预期响应 | 验证要点 |
|------|---------|---------|-----------|---------|---------|
| PROT-001 | 限流触发 | 使用 `hey -n 200 -c 50 ...` 短时间内发送大量请求 | 429 | `{"title":"Too Many Requests","status":429,"detail":"Rate limit exceeded"}` | 超出限流阈值后返回 429；阈值内的请求正常通过 |
| PROT-002 | 限流恢复 | PROT-001 触发后等待令牌桶补充 | 200 | 正常响应 | 等待窗口结束后请求恢复正常 |
| PROT-003 | 熔断触发 | 连续发送 N 次请求到故障后端，触发错误率阈值 | 503 | `{"title":"Service Unavailable","status":503,"detail":"...circuit breaker..."}` | 达到阈值后立即拒绝，不再尝试调用后端 |
| PROT-004 | 熔断半开恢复 | PROT-003 后等待冷却期，发送探测请求 | 200 或 503 | 如后端已恢复则 200 | 半开状态下放行少量请求进行探测 |
| PROT-005 | 超时触发 | 后端延迟超过 `timeout_ms` 配置值 | 504 | `{"title":"Gateway Timeout","status":504}` | detail 中包含 timeout_ms 的实际配置值 |
| PROT-006 | 并发信号量 | 同时发起超过信号量限制数的请求 | 429 或等待后 200 | 超过并发限制的请求被拒绝或排队 | 确认并发保护生效 |

**限流测试 cURL 示例**:

```bash
# 使用 hey 进行压力测试（需提前安装: brew install hey）
hey -n 200 -c 50 -m POST \
  -H "Content-Type: application/json" \
  -d '{"a":1,"b":2}' \
  http://localhost:8080/gw/api/v1/calculator/add

# 检查返回的状态码分布，应能看到部分 429
```

---

## 4. 沙箱测试用例

### 4.1 会话管理

| 编号 | 用例名称 | 请求方法/路径 | 请求体 | 预期状态码 | 预期响应 | 验证要点 |
|------|---------|--------------|--------|-----------|---------|---------|
| SB-001 | 创建 Mock 会话 | `POST /api/v1/projects/{project_id}/sandbox-sessions` | `{"tenant_id":"test-tenant-1","mode":"mock","config":{},"expires_in_hours":24}` | 201 | SandboxSession 对象 | mode 为 `mock`；expires_at 约为当前时间 +24h |
| SB-002 | 创建 Replay 会话 | `POST /api/v1/projects/{project_id}/sandbox-sessions` | `{"tenant_id":"test-tenant-2","mode":"replay","config":{},"expires_in_hours":12}` | 201 | SandboxSession 对象 | mode 为 `replay` |
| SB-003 | 创建 Proxy 会话 | `POST /api/v1/projects/{project_id}/sandbox-sessions` | `{"tenant_id":"test-tenant-3","mode":"proxy","config":{"read_only":true},"expires_in_hours":8}` | 201 | SandboxSession 对象 | mode 为 `proxy`；config 中包含 read_only |
| SB-004 | 创建会话 — 无效 mode | `POST /api/v1/projects/{project_id}/sandbox-sessions` | `{"tenant_id":"t","mode":"invalid","config":{},"expires_in_hours":1}` | 400 / 422 | 错误响应 | 验证 mode 枚举校验 |
| SB-005 | 列出项目会话 | `GET /api/v1/projects/{project_id}/sandbox-sessions` | - | 200 | JSON 数组，包含该项目下所有会话 | 仅返回指定 project_id 的会话 |
| SB-006 | 列出空会话 | `GET /api/v1/projects/{new_project_id}/sandbox-sessions` | 新建项目无会话 | 200 | 空数组 `[]` | - |
| SB-007 | 删除会话 | `DELETE /api/v1/sandbox-sessions/{id}` | SB-001 的 session id | 204 | 无响应体 | 再次列出应不含该会话 |
| SB-008 | 删除不存在的会话 | `DELETE /api/v1/sandbox-sessions/{random-uuid}` | 随机 UUID | 404 | RFC 7807 错误 | - |

**cURL 示例**:

```bash
# SB-001: 创建 Mock 会话
curl -v -X POST http://localhost:8080/api/v1/projects/{project_id}/sandbox-sessions \
  -H "Content-Type: application/json" \
  -d '{"tenant_id":"test-tenant-1","mode":"mock","config":{},"expires_in_hours":24}'

# SB-005: 列出会话
curl -s http://localhost:8080/api/v1/projects/{project_id}/sandbox-sessions | jq .

# SB-007: 删除会话
curl -v -X DELETE http://localhost:8080/api/v1/sandbox-sessions/{session_id}
```

### 4.2 Mock 模式测试

> 沙箱请求通过 `/sandbox/{*rest}` 路径访问。Mock 模式根据路由的 `response_schema` 生成模拟数据。

| 编号 | 用例名称 | 请求 | 请求头 | 预期状态码 | 预期响应 | 验证要点 |
|------|---------|------|--------|-----------|---------|---------|
| MOCK-001 | Mock 返回 Schema 数据 | `POST /sandbox/api/v1/calculator/add` `{"a":1,"b":2}` | `X-Sandbox-Mode: mock` | 200 | JSON 对象，结构匹配 response_schema | 返回的字段名与 schema 定义一致 |
| MOCK-002 | 无 header 默认 Mock | `POST /sandbox/api/v1/calculator/add` `{"a":1,"b":2}` | 无 X-Sandbox-Mode | 200 | Mock 响应 | 缺失 header 时默认使用 mock 模式 |
| MOCK-003 | Smart Mock — email 字段 | `GET /sandbox/api/v1/order/get-order` | `X-Sandbox-Mode: mock` | 200 | customer_email 字段值包含 `@` | MockLayer 对 email 类字段生成合理数据 |
| MOCK-004 | Fixed Response | `POST /sandbox/api/v1/calculator/add` | `X-Sandbox-Mode: mock` + `X-Sandbox-Session: {session_id}` (config 含 fixed_response) | 200 | 返回 config 中指定的固定值 | 优先使用 session 的 fixed_response 配置 |
| MOCK-005 | Mock — 会话过期静默降级 | 使用过期的 session_id | `X-Sandbox-Mode: mock` + `X-Sandbox-Session: {expired_id}` | 200 | Mock 响应（退回 schema 推断） | 过期会话不导致 500 错误，而是静默降级 |
| MOCK-006 | Mock — 路由不存在 | `GET /sandbox/api/v1/nonexistent` | `X-Sandbox-Mode: mock` | 404 | `{"title":"Not Found","detail":"No route matches..."}` | - |

**cURL 示例**:

```bash
# MOCK-001: 基本 Mock
curl -v -X POST http://localhost:8080/sandbox/api/v1/calculator/add \
  -H "Content-Type: application/json" \
  -H "X-Sandbox-Mode: mock" \
  -d '{"a":1,"b":2}'

# MOCK-002: 默认 Mock（无 header）
curl -v -X POST http://localhost:8080/sandbox/api/v1/calculator/add \
  -H "Content-Type: application/json" \
  -d '{"a":1,"b":2}'

# MOCK-004: 带会话的 Fixed Response
curl -v -X POST http://localhost:8080/sandbox/api/v1/calculator/add \
  -H "Content-Type: application/json" \
  -H "X-Sandbox-Mode: mock" \
  -H "X-Sandbox-Session: {session_id}" \
  -d '{"a":1,"b":2}'
```

### 4.3 Replay 模式测试

> Replay 模式从已录制的交互记录中匹配并回放响应。必须提供 `X-Sandbox-Session` 头。

| 编号 | 用例名称 | 请求 | 请求头 | 预期状态码 | 预期响应 | 验证要点 |
|------|---------|------|--------|-----------|---------|---------|
| REPLAY-001 | 无录制返回 404 | `POST /sandbox/api/v1/calculator/add` `{"a":1,"b":2}` | `X-Sandbox-Mode: replay` + `X-Sandbox-Session: {session_id}` | 404 | 无匹配的录制 | 新建 replay 会话尚无录制数据时返回 404 |
| REPLAY-002 | 先录制再回放 | 步骤1: proxy 模式调用 → 自动录制；步骤2: replay 模式调用同路由 | `X-Sandbox-Mode: replay` + `X-Sandbox-Session: {session_id}` | 200 | 与 proxy 录制时的响应一致 | 请求→录制→回放完整链路 |
| REPLAY-003 | 无 session header | `POST /sandbox/api/v1/calculator/add` | `X-Sandbox-Mode: replay`（无 X-Sandbox-Session） | 400 | `{"title":"Bad Request","detail":"X-Sandbox-Session header required for replay mode"}` | 明确提示缺少 session header |
| REPLAY-004 | 无效 session UUID | `POST /sandbox/api/v1/calculator/add` | `X-Sandbox-Mode: replay` + `X-Sandbox-Session: not-a-uuid` | 400 / 404 | 错误响应 | UUID 解析失败时 session_id 为 None，触发 "header required" 错误 |

### 4.4 Proxy 模式测试

> Proxy 模式将请求转发到真实后端，同时可录制交互。必须提供 `X-Sandbox-Session` 头。

| 编号 | 用例名称 | 请求 | 请求头 | 预期状态码 | 预期响应 | 验证要点 |
|------|---------|------|--------|-----------|---------|---------|
| PROXY-001 | 代理到真实后端 | `POST /sandbox/api/v1/calculator/add` `{"a":3,"b":4}` | `X-Sandbox-Mode: proxy` + `X-Sandbox-Session: {session_id}` | 200 | 真实后端返回的结果 | 响应与直接通过 `/gw/` 调用一致 |
| PROXY-002 | 无 session header | `POST /sandbox/api/v1/calculator/add` | `X-Sandbox-Mode: proxy`（无 Session） | 400 | `{"title":"Bad Request","detail":"X-Sandbox-Session header required for proxy mode"}` | - |
| PROXY-003 | 后端不可达 | `POST /sandbox/api/v1/calculator/add` | proxy 模式，后端关闭 | 502 | Bad Gateway | 错误透传 |
| PROXY-004 | 无效沙箱模式 | `POST /sandbox/api/v1/calculator/add` | `X-Sandbox-Mode: invalid_mode` | 400 | `{"detail":"Invalid sandbox mode: 'invalid_mode'. Valid values: mock, replay, proxy"}` | 错误消息列出所有有效值 |
| PROXY-005 | 请求体 > 10MB | `POST /sandbox/...` body 超 10MB | proxy 模式 | 400 | `{"detail":"Failed to read body: ..."}` | 沙箱与网关共享 10MB 限制 |

---

## 5. 补偿机制测试用例

### 5.1 投递保障

> `delivery_guarantee` 有三个级别: `at_most_once`（发即忘）、`at_least_once`（至少一次）、`exactly_once`（恰好一次）。

| 编号 | 用例名称 | 操作步骤 | 预期结果 | 验证方法 |
|------|---------|---------|---------|---------|
| COMP-001 | at_most_once 不记录 | 1. 确保路由的 delivery_guarantee 为 at_most_once 2. 通过 /gw/ 发送请求 | 请求处理成功，但不写入 delivery_records | 查数据库: `SELECT * FROM delivery_records WHERE route_id = '{route_id}'` 应返回空 |
| COMP-002 | at_least_once 创建记录 | 1. 将路由的 delivery_guarantee 改为 at_least_once 2. 发送请求 | 请求成功，delivery_records 中有一条记录，status=delivered | 查数据库确认记录存在且 status=delivered |
| COMP-003 | exactly_once 需要幂等键 | 1. 将路由 delivery_guarantee 改为 exactly_once 2. 不带 Idempotency-Key 头发送请求 | 400 错误 | 响应应提示缺少 Idempotency-Key |
| COMP-004 | exactly_once 正常投递 | 带 `Idempotency-Key: unique-key-001` 头发送请求 | 200，正常响应 | delivery_records 和 idempotency_keys 表中均有记录 |
| COMP-005 | 重复幂等键返回 200 | 用同一 `Idempotency-Key: unique-key-001` 再次发送 | 200，`{"status":"already_delivered"}` | 不会重复执行后端调用；响应明确标识已投递 |
| COMP-006 | 不同幂等键 | 用 `Idempotency-Key: unique-key-002` 发送 | 200，正常新响应 | delivery_records 中多一条记录 |

**cURL 示例**:

```bash
# COMP-004: ExactlyOnce 正常投递
curl -v -X POST http://localhost:8080/gw/api/v1/calculator/add \
  -H "Content-Type: application/json" \
  -H "Idempotency-Key: unique-key-001" \
  -d '{"a":10,"b":20}'

# COMP-005: 重复幂等键
curl -v -X POST http://localhost:8080/gw/api/v1/calculator/add \
  -H "Content-Type: application/json" \
  -H "Idempotency-Key: unique-key-001" \
  -d '{"a":10,"b":20}'
# 预期响应: {"status":"already_delivered"}
```

### 5.2 重试机制

| 编号 | 用例名称 | 操作步骤 | 预期结果 | 验证方法 |
|------|---------|---------|---------|---------|
| RETRY-001 | 失败后自动重试 | 1. 路由 delivery_guarantee 为 at_least_once 2. 后端临时故障（关闭后快速恢复） 3. 发送请求 | 请求返回错误，但 delivery_records 中记录 status=failed 且 next_retry_at 有值 | 查数据库: `next_retry_at` 约为请求时间 +1 秒 |
| RETRY-002 | 指数退避间隔 | 1. 后端持续故障 2. retry_worker 多次重试 | delivery_records 中 retry_count 递增，next_retry_at 间隔递增 | 对比连续几次的 next_retry_at 差值：1s → 2s → 4s → 8s... |
| RETRY-003 | 超限进入死信 | 1. 后端持续故障超过最大重试次数（默认 5 次） | delivery_records 中记录 status=dead | `SELECT status FROM delivery_records WHERE id = '...'` 返回 `dead` |
| RETRY-004 | 成功后不再重试 | 1. 第 2 次重试时后端恢复 | 记录 status 从 failed 变为 delivered，retry_count=2 | next_retry_at 变为 NULL |

### 5.3 管理 API

| 编号 | 用例名称 | 请求方法/路径 | 请求体 / 参数 | 预期状态码 | 预期响应 | 验证要点 |
|------|---------|--------------|--------------|-----------|---------|---------|
| MGMT-001 | 查看死信列表（默认） | `GET /api/v1/compensation/dead-letters` | 无参数 | 200 | JSON 数组，每条含 id、route_id、status、retry_count、error_message | 默认 limit=50, offset=0 |
| MGMT-002 | 按路由过滤死信 | `GET /api/v1/compensation/dead-letters?route_id={uuid}` | route_id 参数 | 200 | 仅包含指定路由的死信 | 数组中所有条目的 route_id 与参数一致 |
| MGMT-003 | 分页查询 | `GET /api/v1/compensation/dead-letters?limit=10&offset=5` | limit + offset | 200 | 最多 10 条，从第 6 条开始 | 配合总数验证分页正确 |
| MGMT-004 | 查看单条投递记录 | `GET /api/v1/compensation/delivery-records/{id}` | delivery_record id | 200 | 完整 DeliveryRecord 对象 | 含 request_payload、response_payload、error_message |
| MGMT-005 | 查看不存在的投递记录 | `GET /api/v1/compensation/delivery-records/{random-uuid}` | 随机 UUID | 404 | RFC 7807 错误 | - |
| MGMT-006 | 单条重推 | `POST /api/v1/compensation/dead-letters/{id}/retry` | 死信记录 id | 204 | 无响应体 | 重推后记录 status 应从 dead 变为 failed，next_retry_at 为当前时间 |
| MGMT-007 | 重推不存在的死信 | `POST /api/v1/compensation/dead-letters/{random-uuid}/retry` | 随机 UUID | 404 | RFC 7807 错误 | - |
| MGMT-008 | 批量重推 | `POST /api/v1/compensation/dead-letters/batch-retry` | `{"ids":["uuid1","uuid2","uuid3"]}` | 200 | `{"retried": N}` | N 为实际成功重置的数量；部分 id 不存在时不影响其他 |
| MGMT-009 | 批量重推 — 空数组 | `POST /api/v1/compensation/dead-letters/batch-retry` | `{"ids":[]}` | 200 | `{"retried": 0}` | - |
| MGMT-010 | 标记已处理 | `POST /api/v1/compensation/dead-letters/{id}/resolve` | 死信记录 id | 204 | 无响应体 | 记录 status 变为 delivered，不再出现在死信列表中 |
| MGMT-011 | 标记不存在的死信 | `POST /api/v1/compensation/dead-letters/{random-uuid}/resolve` | 随机 UUID | 404 | RFC 7807 错误 | - |

**cURL 示例**:

```bash
# MGMT-001: 列出死信
curl -s http://localhost:8080/api/v1/compensation/dead-letters | jq .

# MGMT-003: 分页
curl -s "http://localhost:8080/api/v1/compensation/dead-letters?limit=10&offset=0" | jq .

# MGMT-006: 单条重推
curl -v -X POST http://localhost:8080/api/v1/compensation/dead-letters/{id}/retry

# MGMT-008: 批量重推
curl -v -X POST http://localhost:8080/api/v1/compensation/dead-letters/batch-retry \
  -H "Content-Type: application/json" \
  -d '{"ids":["uuid1","uuid2"]}'

# MGMT-010: 标记已处理
curl -v -X POST http://localhost:8080/api/v1/compensation/dead-letters/{id}/resolve
```

---

## 6. 前端 Web 测试用例

> 前端为 React SPA，使用 React Router 实现客户端路由。未匹配的路径回退到 `index.html`。

### 6.1 Dashboard 页面 (`/`)

| 编号 | 用例名称 | 操作步骤 | 预期结果 | 验证要点 |
|------|---------|---------|---------|---------|
| WEB-001 | 页面加载 — 有项目 | 1. 确保数据库中有项目 2. 浏览器访问 `http://localhost:8080/` | 显示 "Projects" 标题和项目卡片网格 | 每个卡片显示 name、source_type 标签（蓝色）、description、owner |
| WEB-002 | 页面加载 — 无项目 | 1. 清空数据库 2. 访问 `/` | 显示 "No projects yet. Create one to get started." 提示 | 不显示空白页面 |
| WEB-003 | 创建项目 | 1. 点击 "New Project" 按钮 2. 填写 Name、Owner（必填）、Description、选择 Source Type 3. 点击 "Create" | 表单消失，项目列表自动刷新，新项目出现 | Source Type 下拉含 WSDL、CLI、SSH、PTY 四个选项 |
| WEB-004 | 创建项目 — 取消 | 1. 点击 "New Project" 2. 点击 "Cancel" | 表单消失，列表无变化 | - |
| WEB-005 | 创建项目 — 必填校验 | 1. 点击 "New Project" 2. 不填 Name 直接点 "Create" | 浏览器原生表单校验阻止提交 | input 有 `required` 属性 |
| WEB-006 | 删除项目 | 1. 点击项目卡片上的 "Delete" 2. 确认对话框选择 "确定" | 项目从列表消失 | 触发 `confirm()` 对话框 |
| WEB-007 | 删除项目 — 取消 | 1. 点击 "Delete" 2. 确认对话框选择 "取消" | 列表无变化 | - |
| WEB-008 | Source Type 标签颜色 | 查看不同类型项目的标签 | 所有标签使用蓝色底色（bg-blue-100 text-blue-800） | - |
| WEB-009 | 响应式布局 | 调整浏览器宽度 | 小屏 1 列 → 中屏 2 列 → 大屏 3 列 | 使用 grid-cols-1 md:grid-cols-2 lg:grid-cols-3 |

### 6.2 API 文档页面 (`/docs`)

| 编号 | 用例名称 | 操作步骤 | 预期结果 | 验证要点 |
|------|---------|---------|---------|---------|
| WEB-010 | Swagger UI 加载 | 1. 访问 `/docs` 页面 2. 默认显示 Swagger UI tab | iframe 加载 Swagger UI，展示 OpenAPI 规范 | iframe src 指向 `/api/v1/docs` |
| WEB-011 | Swagger UI — 路由展示 | 在 Swagger UI 中展开一个 API | 显示 method、path、request body schema、response schema | 路径以 `/gw/` 前缀开头 |
| WEB-012 | 切换到 Agent Prompt | 1. 点击 "Agent Prompt" tab | 加载并显示 Markdown 格式的提示词 | 内容包含 "# API-Anything Gateway" 和路由列表 |
| WEB-013 | Agent Prompt — 无路由 | 数据库中无激活路由 | 显示 "No routes configured yet." | - |
| WEB-014 | 下载 OpenAPI JSON | 点击右上角 "Download OpenAPI JSON" 按钮 | 浏览器下载 `openapi.json` 文件 | 文件内容为合法 JSON，包含 openapi 3.0.3 字段 |
| WEB-015 | Tab 切换状态 | 在 Swagger UI 和 Agent Prompt 间切换 | 激活 tab 为蓝色（bg-blue-600 text-white），非激活为灰色 | - |

### 6.3 沙箱管理页面 (`/sandbox`)

| 编号 | 用例名称 | 操作步骤 | 预期结果 | 验证要点 |
|------|---------|---------|---------|---------|
| WEB-016 | 页面加载 | 访问 `/sandbox` | 显示 "Sandbox Manager" 标题和 "Select Project" 下拉 | 下拉中列出所有项目 |
| WEB-017 | 选择项目 | 从下拉中选择一个项目 | 显示 "Sessions" 标题和 "New Session" 按钮 | 自动加载该项目的会话列表 |
| WEB-018 | 选择项目 — 无会话 | 选择一个无会话的项目 | 显示 "No sandbox sessions for this project." | - |
| WEB-019 | 创建会话 | 1. 点击 "New Session" 2. 填写 Tenant ID（必填）、选择 Mode（mock/replay/proxy）、填写 Expires in hours、Config JSON 3. 点击 "Create" | 表单消失，会话列表刷新，新会话出现 | - |
| WEB-020 | 会话卡片信息 | 查看会话卡片 | 显示 session ID（等宽字体）、mode 标签（mock=绿、replay=黄、proxy=紫）、Tenant、Expires 时间 | 标签颜色正确 |
| WEB-021 | 查看 cURL 示例 | 展开会话卡片的 "cURL Example" details | 显示包含正确 session id 和 mode 的 curl 命令 | 命令包含 `X-Sandbox-Mode` 和 `X-Sandbox-Session` 头 |
| WEB-022 | 删除会话 | 1. 点击 "Delete" 2. 确认对话框选择 "确定" | 会话从列表消失 | - |
| WEB-023 | Config JSON 容错 | 创建会话时 Config JSON 输入非法 JSON（如 `{invalid`） | 会话仍可创建，config 降级为空对象 `{}` | 前端代码中 `JSON.parse` 失败时 fallback 为 `{}` |

### 6.4 补偿管理页面 (`/compensation`)

| 编号 | 用例名称 | 操作步骤 | 预期结果 | 验证要点 |
|------|---------|---------|---------|---------|
| WEB-024 | 页面加载 — 有死信 | 访问 `/compensation`，数据库中有死信记录 | 表格显示死信列表 | 列：复选框、ID、Route、Status、Retries、Error、Updated、Actions |
| WEB-025 | 页面加载 — 无死信 | 清空死信记录后访问 | 表格中显示 "No dead letters. All clear!" | 居中显示 |
| WEB-026 | 查看 payload | 点击死信 ID（蓝色链接，显示前 8 位） | 表格下方展开 Payload 区域，显示 JSON 格式的 request_payload | JSON 格式化缩进显示 |
| WEB-027 | 单条重推 | 点击某行的 "Retry" 按钮 | 列表自动刷新 | 刷新后该条目的 status 应变化 |
| WEB-028 | 单条标记已处理 | 点击某行的 "Resolve" 按钮 | 列表自动刷新，该条目消失或 status 变为 delivered | - |
| WEB-029 | 勾选单条 | 点击某行复选框 | 顶部出现 "Retry Selected (1)" 按钮（橙色） | 按钮文本包含选中数量 |
| WEB-030 | 全选 | 点击表头的复选框 | 所有行被选中，"Retry Selected" 按钮显示总数 | - |
| WEB-031 | 全选后取消 | 再次点击表头复选框 | 所有选中取消，"Retry Selected" 按钮消失 | - |
| WEB-032 | 批量重推 | 1. 勾选多条 2. 点击 "Retry Selected" | 弹出 alert 显示 "Retried N items"，列表刷新 | N 为实际重置数量 |
| WEB-033 | 手动刷新 | 点击 "Refresh" 按钮 | 列表重新加载 | 显示 Loading... 后更新 |

---

## 7. 前端测试截图说明

> 以下表格定义了每个关键页面/操作需要截取的截图。测试人员执行对应用例时，按编号保存截图（如 `SS-001.png`），附在测试报告中。

### 7.1 Dashboard 页面截图

| 截图编号 | 页面/操作 | 预期显示内容 | 关键 UI 元素标注 |
|---------|----------|-------------|-----------------|
| SS-001 | Dashboard — 项目列表 | 3 列网格布局，每张卡片含项目信息 | [A] "Projects" 标题 [B] "New Project" 蓝色按钮（右上） [C] 项目卡片：名称（粗体）、source_type 标签（蓝色小字）、描述（灰色）、Owner（左下）、Delete 链接（红色，右下） |
| SS-002 | Dashboard — 创建项目表单 | 展开的白色表单区域 | [A] Name 输入框 [B] Owner 输入框 [C] Description 输入框（跨两列） [D] Source Type 下拉选择 [E] "Create" 绿色按钮 [F] "Cancel" 灰色按钮 |
| SS-003 | Dashboard — 空状态 | 无项目的提示文本 | [A] "No projects yet. Create one to get started." 灰色文本 |
| SS-004 | Dashboard — 删除确认 | 浏览器原生 confirm 对话框 | [A] "Delete this project?" 提示文本 [B] 确定/取消按钮 |

### 7.2 API 文档页面截图

| 截图编号 | 页面/操作 | 预期显示内容 | 关键 UI 元素标注 |
|---------|----------|-------------|-----------------|
| SS-005 | API Docs — Swagger UI | Swagger UI 在 iframe 中展示 | [A] "API Documentation" 标题 [B] "Swagger UI" tab（蓝色激活态） [C] "Agent Prompt" tab（灰色） [D] "Download OpenAPI JSON" 按钮（右侧） [E] Swagger UI iframe（显示 API 路由列表，按 protocol tag 分组） |
| SS-006 | API Docs — Agent Prompt | Markdown 格式的提示词 | [A] "Agent Prompt" tab（蓝色激活态） [B] 等宽字体预格式化文本区 [C] 路由信息：method、path、protocol、request/response schema 代码块 |
| SS-007 | API Docs — 无路由 | Agent Prompt 的空状态 | [A] "No routes configured yet." 文本 |

### 7.3 沙箱管理页面截图

| 截图编号 | 页面/操作 | 预期显示内容 | 关键 UI 元素标注 |
|---------|----------|-------------|-----------------|
| SS-008 | Sandbox — 项目选择 | 页面初始状态 | [A] "Sandbox Manager" 标题 [B] "Select Project" 标签 [C] 项目下拉选择框（含 "-- Select --" 占位符） |
| SS-009 | Sandbox — 会话列表 | 选择项目后的会话列表 | [A] "Sessions" 副标题 [B] "New Session" 蓝色按钮 [C] 会话卡片：session ID（等宽字体）、mode 标签（绿/黄/紫）、Tenant 和 Expires 信息、"Delete" 红色链接、"cURL Example" 可展开区域 |
| SS-010 | Sandbox — 创建会话表单 | 新建会话的表单 | [A] Tenant ID 输入框 [B] Mode 下拉（Mock/Replay/Proxy） [C] Expires in hours 数字输入 [D] Config JSON 输入框 [E] "Create" 绿色按钮 [F] "Cancel" 灰色按钮 |
| SS-011 | Sandbox — cURL 示例展开 | 展开 details 后 | [A] "cURL Example" 蓝色链接（已展开） [B] 灰色背景代码块，显示完整 curl 命令，包含 X-Sandbox-Mode 和 X-Sandbox-Session 头 |
| SS-012 | Sandbox — 空会话 | 选中项目但无会话 | [A] "No sandbox sessions for this project." 灰色提示 |

### 7.4 补偿管理页面截图

| 截图编号 | 页面/操作 | 预期显示内容 | 关键 UI 元素标注 |
|---------|----------|-------------|-----------------|
| SS-013 | Compensation — 死信列表 | 有死信记录的表格视图 | [A] "Dead Letter Queue" 标题 [B] "Refresh" 灰色按钮 [C] 表头：复选框（全选）、ID、Route、Status、Retries、Error、Updated、Actions [D] 数据行：ID 显示前 8 位（蓝色链接）、Status 红色标签、Error 截断显示、"Retry" 橙色链接、"Resolve" 绿色链接 |
| SS-014 | Compensation — 展开 Payload | 点击 ID 后 | [A] 被点击的 ID 行高亮 [B] 表格下方 "Payload" 区域 [C] JSON 格式化显示的 request_payload，白色背景、灰色边框 |
| SS-015 | Compensation — 批量操作 | 勾选多条后 | [A] 行复选框被勾选 [B] "Retry Selected (N)" 橙色按钮出现在标题栏右侧 |
| SS-016 | Compensation — 空状态 | 无死信记录 | [A] 表格中居中显示 "No dead letters. All clear!" |
| SS-017 | Compensation — 批量重推结果 | 点击 "Retry Selected" 后 | [A] 浏览器 alert 弹窗显示 "Retried N items" |

---

## 附录 A: 全部 API 路由速查表

| 方法 | 路径 | 说明 | 认证 |
|------|------|------|------|
| GET | `/health` | 存活探针 | 无 |
| GET | `/health/ready` | 就绪探针（含 DB 检查） | 无 |
| POST | `/api/v1/projects` | 创建项目 | 无 |
| GET | `/api/v1/projects` | 列出所有项目 | 无 |
| GET | `/api/v1/projects/{id}` | 获取项目详情 | 无 |
| DELETE | `/api/v1/projects/{id}` | 删除项目 | 无 |
| POST | `/api/v1/projects/{project_id}/sandbox-sessions` | 创建沙箱会话 | 无 |
| GET | `/api/v1/projects/{project_id}/sandbox-sessions` | 列出项目沙箱会话 | 无 |
| DELETE | `/api/v1/sandbox-sessions/{id}` | 删除沙箱会话 | 无 |
| GET | `/api/v1/compensation/dead-letters` | 列出死信队列 | 无 |
| POST | `/api/v1/compensation/dead-letters/batch-retry` | 批量重推死信 | 无 |
| POST | `/api/v1/compensation/dead-letters/{id}/retry` | 单条重推死信 | 无 |
| POST | `/api/v1/compensation/dead-letters/{id}/resolve` | 标记死信已处理 | 无 |
| GET | `/api/v1/compensation/delivery-records/{id}` | 查看投递记录 | 无 |
| GET | `/api/v1/docs` | Swagger UI 页面 | 无 |
| GET | `/api/v1/docs/openapi.json` | OpenAPI 3.0 JSON 规范 | 无 |
| GET | `/api/v1/docs/agent-prompt` | AI Agent 提示词 | 无 |
| ANY | `/gw/{*rest}` | 网关代理（动态路由匹配） | 无 |
| ANY | `/sandbox/{*rest}` | 沙箱代理（mock/replay/proxy） | 无 |

## 附录 B: 错误状态码与含义对照

| 状态码 | 标题 | 触发场景 | AppError 变体 |
|--------|------|---------|--------------|
| 200 | OK / Already Delivered | 正常响应 / 幂等键已投递 | - / `AlreadyDelivered` |
| 201 | Created | 资源创建成功 | - |
| 204 | No Content | 删除成功 / 重推成功 | - |
| 400 | Bad Request | 参数缺失、格式错误、无效沙箱模式 | `BadRequest` |
| 404 | Not Found | 资源不存在、路由不匹配 | `NotFound` |
| 429 | Too Many Requests | 限流触发 | `RateLimited` |
| 500 | Internal Server Error | 数据库异常、dispatcher 缺失 | `Database` / `Internal` |
| 502 | Bad Gateway | 后端不可达、SOAP Fault、后端错误 | `BackendUnavailable` / `BackendError` |
| 503 | Service Unavailable | 熔断器打开 | `CircuitBreakerOpen` |
| 504 | Gateway Timeout | 后端响应超时 | `BackendTimeout` |

## 附录 C: 测试数据文件清单

| 文件路径 | 类型 | 用途 |
|---------|------|------|
| `crates/generator/tests/fixtures/calculator.wsdl` | WSDL | 简单计算器服务（2 操作） |
| `docs/test-data/complex-order-service.wsdl` | WSDL | 复杂订单服务（5 操作，嵌套类型，数组） |
| `crates/generator/tests/fixtures/sample_help.txt` | CLI Help | 简单 report-gen 工具主命令帮助 |
| `crates/generator/tests/fixtures/sample_subcommand_help.txt` | CLI Help | report-gen generate 子命令帮助 |
| `docs/test-data/complex-cli-help.txt` | CLI Help | 复杂 dbctl 工具主命令帮助（10 子命令） |
| `docs/test-data/complex-cli-subcommand-help.txt` | CLI Help | dbctl query 子命令帮助 |
| `crates/generator/tests/fixtures/mock-report-gen.sh` | Shell | 模拟 report-gen CLI（generate/list） |
| `docs/test-data/mock-db-tool.sh` | Shell | 模拟 dbctl CLI（query/status/backup/users） |
| `crates/generator/tests/fixtures/ssh_sample.txt` | SSH Sample | 简单交换机命令（3 命令） |
| `docs/test-data/network-switch-ssh.txt` | SSH Sample | Cisco 9300 交换机（6 命令） |
| `docs/test-data/server-management-ssh.txt` | SSH Sample | Linux 服务器运维（7 命令） |

---

## 8. Phase 6 功能测试用例

### 8.1 Webhook 管理测试

| 编号 | 用例名称 | 请求方法/路径 | 预期状态码 | 验证要点 |
|------|---------|--------------|-----------|---------|
| WH-001 | 创建 Webhook 订阅 | `POST /api/v1/webhooks` `{"url":"https://example.com/hook","event_types":["DeadLetter"],"description":"test"}` | 201 | 返回完整 Webhook 对象，含 `id`、`url`、`event_types` |
| WH-002 | 列出订阅 | `GET /api/v1/webhooks` | 200 | 返回 JSON 数组，包含已创建的订阅 |
| WH-003 | 删除订阅 | `DELETE /api/v1/webhooks/{id}` | 204 | 无响应体；再次 GET 列表应不含该订阅 |

**cURL 示例**:

```bash
# WH-001: 创建订阅
curl -v -X POST http://localhost:8080/api/v1/webhooks \
  -H "Content-Type: application/json" \
  -d '{"url":"https://example.com/hook","event_types":["DeadLetter","DeliveryFailed"],"description":"告警通知"}'

# WH-002: 列出订阅
curl -s http://localhost:8080/api/v1/webhooks | jq .

# WH-003: 删除订阅
curl -v -X DELETE http://localhost:8080/api/v1/webhooks/{id}
```

### 8.2 SDK 生成测试

| 编号 | 用例名称 | 请求方法/路径 | 预期状态码 | 验证要点 |
|------|---------|--------------|-----------|---------|
| SDK-001 | TypeScript SDK | `GET /api/v1/docs/sdk/typescript` | 200 | 响应体包含 `fetch` 关键字 |
| SDK-002 | Python SDK | `GET /api/v1/docs/sdk/python` | 200 | 响应体包含 `requests` 关键字 |
| SDK-003 | 不支持的语言 | `GET /api/v1/docs/sdk/cobol` | 400 | 错误响应，提示不支持的语言 |

**cURL 示例**:

```bash
# SDK-001: TypeScript SDK
curl -s http://localhost:8080/api/v1/docs/sdk/typescript | head -20

# SDK-002: Python SDK
curl -s http://localhost:8080/api/v1/docs/sdk/python | head -20

# SDK-003: 不支持的语言
curl -v http://localhost:8080/api/v1/docs/sdk/cobol
```

### 8.3 录制管理测试

| 编号 | 用例名称 | 请求方法/路径 | 预期状态码 | 验证要点 |
|------|---------|--------------|-----------|---------|
| REC-001 | 查看录制 | `GET /api/v1/sandbox-sessions/{id}/recordings` | 200 | 返回 JSON 数组，每条含请求和响应数据 |
| REC-002 | 清空录制 | `DELETE /api/v1/sandbox-sessions/{id}/recordings` | 204 | 无响应体；再次 GET 应返回空数组 |

**cURL 示例**:

```bash
# REC-001: 查看录制
curl -s http://localhost:8080/api/v1/sandbox-sessions/{session_id}/recordings | jq .

# REC-002: 清空录制
curl -v -X DELETE http://localhost:8080/api/v1/sandbox-sessions/{session_id}/recordings
```

### 8.4 插件管理测试

| 编号 | 用例名称 | 请求方法/路径 | 预期状态码 | 验证要点 |
|------|---------|--------------|-----------|---------|
| PLG-001 | 列出插件 | `GET /api/v1/plugins` | 200 | 返回 JSON 数组，含已加载插件列表 |
| PLG-002 | 扫描插件 | `POST /api/v1/plugins/scan` | 200 | 返回扫描结果，含发现和加载的插件数量 |

**cURL 示例**:

```bash
# PLG-001: 列出插件
curl -s http://localhost:8080/api/v1/plugins | jq .

# PLG-002: 扫描插件目录
curl -v -X POST http://localhost:8080/api/v1/plugins/scan
```

### 8.5 前端新页面测试

| 编号 | 用例名称 | 操作步骤 | 预期结果 | 验证要点 |
|------|---------|---------|---------|---------|
| WEB-014 | API Explorer 加载 | 浏览器访问 `/explorer` | 显示路由列表和请求构建器 | 左侧显示按项目分组的路由列表；右侧显示请求面板 |
| WEB-015 | Webhook Manager | 浏览器访问 `/webhooks` | 显示订阅表格 | 表格包含 URL、Event Types、Description、Actions 列 |
| WEB-016 | SDK Generator | 在 `/docs` 页面切换到 SDK Generator tab | 显示代码预览区域 | 语言下拉列表含 TypeScript、Python、Java、Go 四个选项 |
| WEB-017 | Monitoring | 浏览器访问 `/monitoring` | 显示 Grafana iframe | iframe 正确嵌入 Grafana 仪表盘；需 Grafana 服务运行 |
