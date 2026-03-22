# API-Anything 全流程全链路测试文档

> 面向 QA 和开发团队 | 版本 1.0 | 2026-03-22

---

## 目录

1. [测试策略概述](#1-测试策略概述)
2. [全链路测试场景](#2-全链路测试场景)
3. [保护层测试场景](#3-保护层测试场景)
4. [沙箱全链路测试](#4-沙箱全链路测试)
5. [补偿机制全链路测试](#5-补偿机制全链路测试)
6. [文档服务测试](#6-文档服务测试)
7. [前端 Web 全链路测试](#7-前端-web-全链路测试)
8. [性能和稳定性测试](#8-性能和稳定性测试)
9. [安全测试](#9-安全测试)
10. [测试执行报告模板](#10-测试执行报告模板)
11. [前端测试截图清单](#11-前端测试截图清单)
12. [Phase 6 全链路测试场景](#12-phase-6-全链路测试场景)

---

## 1. 测试策略概述

### 1.1 测试目标

- 验证从原始契约输入（WSDL/CLI/SSH/PTY）到最终 REST API 响应的完整链路正确性
- 验证网关保护层（限流、熔断、并发控制）在正常和异常场景下的行为符合预期
- 验证沙箱三层模式（Mock/Replay/Proxy）的隔离性和数据正确性
- 验证补偿引擎的投递保障（at_least_once/exactly_once）和死信处理流程
- 验证系统在高并发、长时间运行和故障恢复场景下的稳定性
- 验证所有错误响应符合 RFC 7807 规范且不泄露敏感信息

### 1.2 测试范围

| 范围 | 说明 |
|------|------|
| 适配器层 | SOAP、CLI、SSH、PTY 四种协议适配器的请求转换、后端执行、响应解析 |
| 保护层 | 令牌桶限流（RateLimiter）、三态熔断器（CircuitBreaker）、并发信号量（ConcurrencySemaphore） |
| 沙箱引擎 | MockLayer 数据生成、ReplayLayer 交互回放、ProxyLayer 透传与隔离、Recorder 录制 |
| 补偿引擎 | RequestLogger 请求记录、RetryWorker 指数退避重试、IdempotencyGuard 幂等保障、DeadLetterProcessor 死信处理 |
| Platform API | 项目 CRUD、沙箱会话管理、补偿管理 API、文档端点、网关动态路由、健康检查 |
| 前端 Web | 项目管理、沙箱管理、补偿管理、API 文档浏览（React SPA） |

### 1.3 测试环境要求

| 组件 | 要求 |
|------|------|
| 操作系统 | Linux (Ubuntu 22.04+) 或 macOS 14+ |
| Rust | 1.77+ (stable) |
| PostgreSQL | 15+ （含自定义枚举类型：source_type, http_method, contract_status, protocol_type, delivery_guarantee, artifact_type, build_status, delivery_status, sandbox_mode） |
| Kafka | 3.6+ （topic: delivery-events, route-updates, push-events） |
| Docker | 24+ (Compose V2) |
| Node.js | 20+ (前端构建) |
| 网络 | 测试机可访问 SOAP Mock 服务、SSH 测试服务器 |

### 1.4 测试工具

| 工具 | 用途 | 版本 |
|------|------|------|
| curl | 接口功能验证、手动调试 | 8.0+ |
| wrk | HTTP 基准压测 | 4.2+ |
| k6 | 高级负载测试与场景编排 | 0.50+ |
| WireMock | SOAP/HTTP 后端 Mock 服务 | 3.0+ |
| jq | JSON 响应解析与断言 | 1.7+ |
| psql | 数据库状态验证 | 15+ |
| cargo test | Rust 单元/集成测试 | 与 Rust 版本一致 |

### 1.5 测试数据管理策略

| 策略 | 说明 |
|------|------|
| 数据隔离 | 每个测试场景使用独立的 Project + Contract，避免场景间污染 |
| 数据清理 | 每轮测试前执行 `TRUNCATE CASCADE` 清理全部业务表 |
| 种子数据 | 通过 SQL 脚本预置基础数据（项目、契约、路由、后端绑定） |
| 幂等验证 | exactly_once 场景使用带时间戳的唯一 Idempotency-Key |
| WSDL 样本 | 存放于 `docs/test-data/wsdl/`，包含简单、复杂、大型三种规模 |
| CLI 样本 | 存放于 `docs/test-data/cli/`，使用系统内置命令（echo、ls、cat）避免外部依赖 |

---

## 2. 全链路测试场景

### 场景 1: SOAP 遗留系统全链路

**链路流程：**

```
WSDL 文件 → CLI generate → 元数据写入（Contract + Route + BackendBinding）→
网关加载路由 → 客户端 JSON 请求 → 网关 /gw/* 路由匹配 →
SoapAdapter.transform_request（JSON→SOAP Envelope）→
SoapAdapter.execute（reqwest POST, SOAPAction Header）→
后端 SOAP 调用 → SOAP XML 响应 →
SoapAdapter.transform_response（SoapXmlParser 解析）→ JSON 转换 → 客户端接收
```

#### 基本流程测试

**步骤 1: 启动 WireMock 模拟 SOAP 后端**

```bash
# 启动 WireMock 并配置 SOAP 端点
wiremock --port 9090 &

# 配置 SOAP 响应映射
curl -X POST http://localhost:9090/__admin/mappings -H "Content-Type: application/json" -d '{
  "request": {
    "method": "POST",
    "url": "/calculator",
    "headers": {
      "SOAPAction": { "equalTo": "http://example.com/calculator/Add" }
    }
  },
  "response": {
    "status": 200,
    "headers": { "Content-Type": "text/xml; charset=utf-8" },
    "body": "<?xml version=\"1.0\"?><soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\"><soap:Body><AddResponse xmlns=\"http://example.com/calculator\"><result>42</result></AddResponse></soap:Body></soap:Envelope>"
  }
}'
```

**步骤 2: 创建项目**

```bash
curl -s -X POST http://localhost:3000/api/v1/projects \
  -H "Content-Type: application/json" \
  -d '{
    "name": "soap-calculator",
    "description": "SOAP Calculator 遗留系统",
    "owner": "team-platform",
    "source_type": "wsdl",
    "source_config": {
      "wsdl_url": "http://localhost:9090/calculator?wsdl",
      "endpoint_url": "http://localhost:9090/calculator"
    }
  }' | jq .
```

**期望输出：**

```json
{
  "id": "<uuid>",
  "name": "soap-calculator",
  "source_type": "wsdl",
  "created_at": "2026-03-22T..."
}
```

**步骤 3: 通过网关调用（元数据注入后）**

```bash
# 向网关发送 JSON 请求，验证 SOAP 转换全链路
curl -s -X POST http://localhost:3000/gw/api/v1/calculator/add \
  -H "Content-Type: application/json" \
  -d '{"a": 20, "b": 22}' | jq .
```

**期望输出：**

```json
{
  "result": "42"
}
```

**步骤 4: 验证 SOAP Envelope 构建**

```bash
# 通过 WireMock 请求日志验证发出的 SOAP 请求
curl -s http://localhost:9090/__admin/requests | jq '.requests[0].body' | \
  grep -q '<soap:Envelope' && echo "PASS: SOAP Envelope 构建正确" || echo "FAIL"
```

#### 复杂场景

##### 1.1 多操作 WSDL（5+ 操作）

- 准备包含 Add、Subtract、Multiply、Divide、Modulo 五个操作的 WSDL
- 验证每个操作生成独立路由（`/gw/api/v1/calculator/add`, `/subtract`, `/multiply` 等）
- 验证不同操作的 SOAPAction 头正确设置

```bash
# 依次验证各操作路由
for op in add subtract multiply divide modulo; do
  STATUS=$(curl -s -o /dev/null -w '%{http_code}' \
    -X POST "http://localhost:3000/gw/api/v1/calculator/${op}" \
    -H "Content-Type: application/json" \
    -d '{"a": 10, "b": 5}')
  echo "Operation ${op}: HTTP ${STATUS}"
done
```

**期望：** 所有操作返回 HTTP 200

##### 1.2 嵌套复杂类型（订单含商品列表）

```bash
curl -s -X POST http://localhost:3000/gw/api/v1/orders \
  -H "Content-Type: application/json" \
  -d '{
    "customer_id": "C001",
    "items": [
      {"product_id": "P001", "quantity": 2, "attributes": {"color": "red", "size": "L"}},
      {"product_id": "P002", "quantity": 1, "attributes": {"color": "blue"}}
    ],
    "shipping_address": {
      "street": "123 Main St",
      "city": "Shanghai",
      "zip": "200000"
    }
  }' | jq .
```

**验证点：**

- 嵌套对象正确序列化为 XML 子元素
- 数组正确展开为多个同名 XML 元素
- 响应中嵌套结构正确还原为 JSON

##### 1.3 SOAP Fault 错误处理

```bash
# 配置 WireMock 返回 SOAP Fault
curl -X POST http://localhost:9090/__admin/mappings -d '{
  "request": { "method": "POST", "url": "/calculator-fault" },
  "response": {
    "status": 500,
    "headers": { "Content-Type": "text/xml" },
    "body": "<?xml version=\"1.0\"?><soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\"><soap:Body><soap:Fault><faultcode>soap:Server</faultcode><faultstring>Division by zero</faultstring></soap:Fault></soap:Body></soap:Envelope>"
  }
}'

# 触发 SOAP Fault
curl -s -X POST http://localhost:3000/gw/api/v1/calculator/divide \
  -H "Content-Type: application/json" \
  -d '{"a": 10, "b": 0}' | jq .
```

**期望输出（RFC 7807 格式）：**

```json
{
  "type": "https://api-anything.dev/errors/backend-error",
  "title": "Backend Error",
  "status": 500,
  "detail": "SOAP Fault: Division by zero"
}
```

##### 1.4 WS-Security Header 注入

- 验证 `BackendBinding.auth_mapping` 配置的 WS-Security 凭证正确注入到 SOAP Header
- 通过 WireMock 请求日志验证 `<wsse:Security>` 元素存在且包含正确的 UsernameToken

##### 1.5 大型 WSDL（1000+ 行，RAG 分块）

- 使用 1000+ 行 WSDL 文件，包含 20+ portType 操作
- 验证 RAG 按 `<wsdl:portType>` 分块后合并结果的一致性
- 验证跨块类型引用（一个操作引用另一个块中定义的复杂类型）正确解析

---

### 场景 2: CLI 工具全链路

**链路流程：**

```
--help 输出 → CLI generate-cli → 元数据写入（Contract + Route + BackendBinding）→
网关加载路由 → JSON 请求 →
CliAdapter.transform_request（JSON→args 列表，bool/null 特殊处理）→
CliAdapter.execute（tokio::process::Command, 逐参数 .arg() 安全传递）→
进程执行 → stdout/stderr 捕获 → exit_code 判断 →
CliAdapter.transform_response（OutputParser 按 output_format 解析）→ JSON 响应
```

#### 基本流程测试

**步骤 1: 创建 CLI 项目**

```bash
curl -s -X POST http://localhost:3000/api/v1/projects \
  -H "Content-Type: application/json" \
  -d '{
    "name": "system-tools",
    "description": "系统工具命令封装",
    "owner": "team-infra",
    "source_type": "cli",
    "source_config": {
      "program": "echo",
      "subcommand": null,
      "static_args": [],
      "output_format": "raw_text"
    }
  }' | jq .
```

**步骤 2: 通过网关调用 CLI 命令**

```bash
curl -s -X POST http://localhost:3000/gw/api/v1/tools/echo \
  -H "Content-Type: application/json" \
  -d '{"message": "hello world"}' | jq .
```

**期望输出：**

```json
{
  "stdout": "hello world\n"
}
```

**步骤 3: 验证参数转换**

```bash
# 验证 JSON 字段正确转为 --key value 格式
curl -s -X POST http://localhost:3000/gw/api/v1/tools/ls \
  -H "Content-Type: application/json" \
  -d '{"format": "long", "all": true, "human_readable": true}' | jq .
```

**验证点：**

- `"format": "long"` 转为 `--format long`
- `"all": true` 转为 `--all`（布尔 flag 无值）
- `"human_readable": true` 转为 `--human_readable`

#### 复杂场景

##### 2.1 子命令嵌套（tool subcmd sub-subcmd）

```bash
# 配置 subcommand 为 "status"，static_args 含 "--verbose"
curl -s -X POST http://localhost:3000/gw/api/v1/tools/git-status \
  -H "Content-Type: application/json" \
  -d '{"short": true}' | jq .
```

**验证：** 实际执行的命令为 `git status --verbose --short`（subcommand 在 flag 之前）

##### 2.2 JSON 输出解析（output_format: json）

```bash
# output_format 设为 "json" 时，stdout 应被解析为 JSON 对象返回
curl -s -X POST http://localhost:3000/gw/api/v1/tools/docker-inspect \
  -H "Content-Type: application/json" \
  -d '{"container": "my-app"}' | jq .
```

**验证：** 返回值为解析后的 JSON 对象，而非包含 JSON 字符串的 `{"stdout": "..."}`

##### 2.3 正则表达式输出提取（output_format: regex）

- 配置正则提取规则（如从 `df -h` 输出提取磁盘使用率）
- 验证 OutputParser 正则匹配后返回结构化数据

##### 2.4 长时间运行命令（验证超时）

```bash
# 配置 timeout_ms 为 2000，执行 sleep 10
curl -s -X POST http://localhost:3000/gw/api/v1/tools/slow-cmd \
  -H "Content-Type: application/json" \
  -d '{}' | jq .
```

**期望：** 返回 504 Gateway Timeout（BackendTimeout）

##### 2.5 命令注入防护验证

```bash
# 尝试注入 shell 命令
curl -s -X POST http://localhost:3000/gw/api/v1/tools/echo \
  -H "Content-Type: application/json" \
  -d '{"message": "; rm -rf /"}' | jq .
```

**期望：** `echo` 命令接收到字面字符串 `; rm -rf /` 作为参数（通过 .arg() 安全传递），stdout 输出 `; rm -rf /\n`，而非执行 `rm` 命令

```bash
# 进一步验证各种注入变体
for payload in '$(whoami)' '`id`' '| cat /etc/passwd' '&& echo hacked'; do
  RESULT=$(curl -s -X POST http://localhost:3000/gw/api/v1/tools/echo \
    -H "Content-Type: application/json" \
    -d "{\"message\": \"${payload}\"}" | jq -r '.stdout')
  echo "Input: ${payload} => Output: ${RESULT}"
done
```

**期望：** 每行输出的 Input 和 Output 字面一致（未被 shell 解释执行）

##### 2.6 进程并发限制验证（10 并发上限）

```bash
# 使用 k6 同时发起 15 个请求（CLI 默认并发上限 10）
k6 run --vus 15 --duration 5s - <<'EOF'
import http from 'k6/http';
import { check } from 'k6';
export default function () {
  const res = http.post('http://localhost:3000/gw/api/v1/tools/slow-echo',
    JSON.stringify({message: "test"}),
    {headers: {'Content-Type': 'application/json'}});
  check(res, {
    'status is 200 or 429/503': (r) => r.status === 200 || r.status === 429 || r.status === 503,
  });
}
EOF
```

**验证：** 前 10 个并发请求返回 200，超出部分被信号量拒绝或排队

---

### 场景 3: SSH 远程命令全链路

**链路流程：**

```
SSH 样例文件 → CLI generate-ssh → 元数据写入 → 网关加载路由 →
JSON 请求 → SshAdapter.transform_request（合并 path_params + body → 命令模板替换）→
SshAdapter.execute（系统 ssh 二进制, BatchMode=yes, StrictHostKeyChecking=accept-new）→
SSH 远程执行 → stdout/stderr + exit_code 捕获 →
SshAdapter.transform_response（exit_code 255→502, 其他非零→500, OutputParser 解析）→ JSON 响应
```

#### 基本流程测试

**前置条件：** 准备 SSH 测试服务器（可用 Docker `linuxserver/openssh-server`）

```bash
# 创建 SSH 项目
curl -s -X POST http://localhost:3000/api/v1/projects \
  -H "Content-Type: application/json" \
  -d '{
    "name": "network-device",
    "description": "网络设备管理",
    "owner": "team-network",
    "source_type": "ssh",
    "source_config": {
      "host": "10.0.1.50",
      "port": 22,
      "user": "admin",
      "command_template": "show interfaces status",
      "output_format": "raw_text"
    }
  }' | jq .

# 调用 SSH 命令
curl -s -X GET http://localhost:3000/gw/api/v1/network/interfaces/status | jq .
```

**期望：** 返回远程命令执行结果的结构化 JSON

#### 复杂场景

##### 3.1 带参数的命令模板

```bash
# command_template: "show running-config interface {interface}"
curl -s -X GET "http://localhost:3000/gw/api/v1/network/interfaces/Gi0%2F1/config" | jq .
```

**验证：** 路径参数 `{interface}` 被替换为 `Gi0/1`，实际执行 `show running-config interface Gi0/1`

```bash
# body 参数覆盖 path 参数
curl -s -X POST "http://localhost:3000/gw/api/v1/network/interfaces/Gi0%2F1/config" \
  -H "Content-Type: application/json" \
  -d '{"interface": "Gi0/2"}' | jq .
```

**验证：** 请求体中的 `interface` 值 `Gi0/2` 覆盖路径参数 `Gi0/1`

##### 3.2 SSH 连接超时

```bash
# 连接不可达主机
curl -s -X GET http://localhost:3000/gw/api/v1/network/unreachable/status | jq .
```

**期望输出：**

```json
{
  "type": "https://api-anything.dev/errors/backend-unavailable",
  "title": "Backend Unavailable",
  "status": 502,
  "detail": "SSH connection error: ..."
}
```

**验证：** SSH exit code 255 映射为 HTTP 502

##### 3.3 SSH 认证失败

- 配置错误的 identity_file 路径
- 验证 BatchMode=yes 防止交互式密码提示导致进程挂起
- 验证返回 502（SSH 连接层错误）而非 500（远端命令错误）

##### 3.4 多跳 SSH (bastion host)

- 通过 ProxyJump 配置跳板机连接
- 验证命令最终在目标机器上执行
- 验证超时设置对多跳场景有效（总超时而非单跳超时）

---

### 场景 4: PTY 交互式会话全链路

**链路流程：**

```
配置（program + prompt_pattern + init_commands + command_template）→ 网关加载 →
JSON 请求 → PtyAdapter.transform_request（合并参数 → 命令模板替换）→
PtyAdapter.execute:
  1. tokio::process::Command spawn（piped stdin/stdout）
  2. 依次执行 init_commands（每条后等待 prompt_pattern 匹配）
  3. 发送实际命令 → 等待 prompt_pattern → 收集输出
  4. child.kill() 清理子进程
→ PtyAdapter.transform_response（OutputParser 解析）→ JSON 响应
```

#### 基本流程测试

```bash
# 创建 PTY 项目
curl -s -X POST http://localhost:3000/api/v1/projects \
  -H "Content-Type: application/json" \
  -d '{
    "name": "db-repl",
    "description": "数据库 REPL 交互封装",
    "owner": "team-data",
    "source_type": "pty",
    "source_config": {
      "program": "bash",
      "args": ["-c", "echo hello PTY; echo PROMPT>"],
      "prompt_pattern": "PROMPT>",
      "command_template": "select * from {table}",
      "output_format": "raw_text",
      "init_commands": [],
      "timeout_ms": 5000
    }
  }' | jq .
```

#### 复杂场景

##### 4.1 多步骤交互（登录 → 模式切换 → 命令 → 退出）

```bash
# init_commands: ["enable", "configure terminal"]
# command_template: "show interface {name}"
# prompt_pattern: "#\\s*$"
curl -s -X GET "http://localhost:3000/gw/api/v1/device/interfaces/eth0" | jq .
```

**验证：**

1. 进程启动后依次发送 `enable` 和 `configure terminal`
2. 每条 init_command 后等待提示符 `#` 出现
3. 发送实际命令 `show interface eth0`
4. 收集提示符出现前的所有输出
5. 提示符本身不出现在返回值中
6. 子进程被 kill 清理

##### 4.2 提示符变化

- 初始提示符 `>`，执行 `enable` 后变为 `#`
- 验证 prompt_pattern 正则能匹配多种提示符格式
- 使用正则 `[>#]\s*$` 覆盖两种模式

##### 4.3 超时处理

```bash
# timeout_ms: 2000，进程长时间无输出
curl -s -X GET "http://localhost:3000/gw/api/v1/device/slow-query" | jq .
```

**期望：**

```json
{
  "type": "https://api-anything.dev/errors/backend-timeout",
  "title": "Backend Timeout",
  "status": 504,
  "detail": "Operation timed out after 2000ms"
}
```

**验证：** 超时后子进程被正确 kill，无进程泄漏

---

## 3. 保护层测试场景

### 场景 5: 限流保护

**组件：** `RateLimiter`（令牌桶算法）

##### 5.1 正常流量通过

```bash
# 配置 requests_per_second=10, burst_size=10
# 以 5 QPS 发送请求，全部应返回 200
for i in $(seq 1 10); do
  STATUS=$(curl -s -o /dev/null -w '%{http_code}' \
    http://localhost:3000/gw/api/v1/calculator/add \
    -X POST -H "Content-Type: application/json" -d '{"a":1,"b":1}')
  echo "Request ${i}: ${STATUS}"
  sleep 0.2
done
```

**期望：** 全部 HTTP 200

##### 5.2 突发流量触发 429

```bash
# burst_size=10，快速发送 15 个请求（不等待令牌补充）
for i in $(seq 1 15); do
  STATUS=$(curl -s -o /dev/null -w '%{http_code}' \
    http://localhost:3000/gw/api/v1/calculator/add \
    -X POST -H "Content-Type: application/json" -d '{"a":1,"b":1}')
  echo "Request ${i}: ${STATUS}"
done
```

**期望：** 前 10 个返回 200，后 5 个返回 429 (RateLimited)

##### 5.3 令牌补充后恢复

```bash
# 耗尽令牌后等待令牌补充（100 req/s → 10ms 补充 1 个令牌）
for i in $(seq 1 10); do
  curl -s -o /dev/null http://localhost:3000/gw/api/v1/calculator/add \
    -X POST -H "Content-Type: application/json" -d '{"a":1,"b":1}'
done
# 全部耗尽
STATUS_REJECT=$(curl -s -o /dev/null -w '%{http_code}' \
  http://localhost:3000/gw/api/v1/calculator/add \
  -X POST -H "Content-Type: application/json" -d '{"a":1,"b":1}')
echo "After exhaust: ${STATUS_REJECT}"  # 期望 429

sleep 0.1  # 等待令牌补充

STATUS_RECOVER=$(curl -s -o /dev/null -w '%{http_code}' \
  http://localhost:3000/gw/api/v1/calculator/add \
  -X POST -H "Content-Type: application/json" -d '{"a":1,"b":1}')
echo "After refill: ${STATUS_RECOVER}"  # 期望 200
```

##### 5.4 不同路由独立限流

```bash
# 路由 A 限流 2 QPS，路由 B 限流 100 QPS
# 耗尽路由 A 的令牌后，路由 B 仍可正常访问
curl -s -o /dev/null http://localhost:3000/gw/api/v1/route-a -X POST -d '{}'
curl -s -o /dev/null http://localhost:3000/gw/api/v1/route-a -X POST -d '{}'
STATUS_A=$(curl -s -o /dev/null -w '%{http_code}' http://localhost:3000/gw/api/v1/route-a -X POST -d '{}')
STATUS_B=$(curl -s -o /dev/null -w '%{http_code}' http://localhost:3000/gw/api/v1/route-b -X POST -d '{}')
echo "Route A (exhausted): ${STATUS_A}"  # 期望 429
echo "Route B (independent): ${STATUS_B}"  # 期望 200
```

---

### 场景 6: 熔断保护

**组件：** `CircuitBreaker`（三态：Closed → Open → HalfOpen）

##### 6.1 正常 → 错误率上升 → 熔断打开 (503)

```bash
# 配置 error_threshold_percent=50, window_duration=30s, half_open_max_requests=3
# 先发送正常请求建立基线，然后让后端持续返回错误

# 阶段 1: 正常请求
for i in $(seq 1 5); do
  curl -s -o /dev/null http://localhost:3000/gw/api/v1/fragile-backend -X POST -d '{}'
done

# 阶段 2: 配置 WireMock 返回 500，触发大量失败
curl -X POST http://localhost:9090/__admin/mappings -d '{
  "request": {"method": "POST", "url": "/fragile"},
  "response": {"status": 500, "body": "Internal Error"}
}'

for i in $(seq 1 20); do
  curl -s -o /dev/null http://localhost:3000/gw/api/v1/fragile-backend -X POST -d '{}'
done

# 阶段 3: 验证熔断器打开
STATUS=$(curl -s -o /dev/null -w '%{http_code}' \
  http://localhost:3000/gw/api/v1/fragile-backend -X POST -d '{}')
echo "Circuit Open: ${STATUS}"  # 期望 503
```

##### 6.2 等待 open_duration → Half-Open

```bash
# open_duration 配置为 5s，等待后验证状态转换
sleep 6

# HalfOpen 状态允许请求通过（试探请求）
STATUS=$(curl -s -o /dev/null -w '%{http_code}' \
  http://localhost:3000/gw/api/v1/fragile-backend -X POST -d '{}')
echo "Half-Open probe: ${STATUS}"  # 期望 200 或 500（取决于后端是否恢复）
```

##### 6.3 试探请求成功 → 恢复

```bash
# 恢复 WireMock 正常响应
curl -X POST http://localhost:9090/__admin/mappings/reset

# 发送 half_open_max_requests (3) 个成功请求
for i in $(seq 1 3); do
  curl -s -o /dev/null http://localhost:3000/gw/api/v1/fragile-backend -X POST -d '{}'
done

# 验证熔断器关闭
STATUS=$(curl -s -o /dev/null -w '%{http_code}' \
  http://localhost:3000/gw/api/v1/fragile-backend -X POST -d '{}')
echo "Circuit Closed: ${STATUS}"  # 期望 200
```

##### 6.4 试探请求失败 → 重新打开

```bash
# HalfOpen 期间出现失败 → 立即重新打开
# （在 HalfOpen 状态下让后端继续返回 500）
STATUS=$(curl -s -o /dev/null -w '%{http_code}' \
  http://localhost:3000/gw/api/v1/fragile-backend -X POST -d '{}')
echo "Re-opened: ${STATUS}"  # 期望 503
```

##### 6.5 不同后端独立熔断

- 后端 A 熔断打开 → 后端 B 不受影响
- 验证 BackendBinding 级别的熔断器隔离

---

### 场景 7: 并发信号量

**组件：** `ConcurrencySemaphore`（tokio Semaphore 封装，RAII 自动归还）

##### 7.1 CLI 后端 10 并发限制

```bash
# 使用 k6 发起 15 并发，CLI 后端默认 max_concurrent=10
k6 run --vus 15 --duration 10s - <<'EOF'
import http from 'k6/http';
import { check, sleep } from 'k6';
export default function () {
  const res = http.post('http://localhost:3000/gw/api/v1/tools/slow-cmd',
    JSON.stringify({}),
    {headers: {'Content-Type': 'application/json'}});
  check(res, {
    'is ok or limited': (r) => r.status === 200 || r.status === 503,
  });
  sleep(0.1);
}
EOF
```

##### 7.2 SSH 后端 5 并发限制

- 同时发起 8 个 SSH 命令
- 验证前 5 个正常执行，后 3 个排队或被拒绝

##### 7.3 PTY 后端 3 并发限制

- 同时发起 5 个 PTY 会话
- 验证前 3 个正常执行，后 2 个排队或被拒绝

##### 7.4 超限请求排队/拒绝

```bash
# 验证 try_acquire（非阻塞）立即返回错误
# 验证 acquire（异步阻塞）在许可释放后获取成功

# 数据库验证信号量状态
psql -c "SELECT * FROM backend_bindings WHERE protocol = 'cli'" | head -5
```

---

## 4. 沙箱全链路测试

### 场景 8: Mock 模式全链路

**组件：** `MockLayer`（Smart Mock + Schema Mock + Fixed Mock）

##### 8.1 基本 Mock 流程

```bash
# 步骤 1: 创建 Mock 沙箱会话
SESSION=$(curl -s -X POST "http://localhost:3000/api/v1/projects/${PROJECT_ID}/sandbox-sessions" \
  -H "Content-Type: application/json" \
  -d '{
    "tenant_id": "qa-team-1",
    "mode": "mock",
    "config": {},
    "expires_at": "2026-03-23T00:00:00Z"
  }' | jq -r '.id')
echo "Session: ${SESSION}"

# 步骤 2: 通过沙箱路由发送请求
curl -s -X POST "http://localhost:3000/sandbox/api/v1/orders" \
  -H "Content-Type: application/json" \
  -H "X-Sandbox-Mode: mock" \
  -H "X-Sandbox-Session: ${SESSION}" \
  -d '{}' | jq .
```

**期望：** 返回根据 Route.response_schema 自动生成的 Mock 数据

##### 8.2 Smart Mock 字段验证

```bash
# response_schema 包含 email、phone、id、date、amount 字段
curl -s -X GET "http://localhost:3000/sandbox/api/v1/users/1" \
  -H "X-Sandbox-Mode: mock" \
  -H "X-Sandbox-Session: ${SESSION}" | jq .
```

**期望输出（Smart Mock 语义推断）：**

```json
{
  "id": "<uuid-格式>",
  "email": "user@example.com",
  "phone": "+86-13800001234",
  "name": "John Doe",
  "amount": 99.50,
  "created_at": "2024-01-15T10:30:00Z",
  "status": "active",
  "description": "Sample description text",
  "url": "https://example.com"
}
```

##### 8.3 枚举值随机选择

```bash
# schema 含 "status": {"type": "string", "enum": ["active", "inactive", "pending"]}
# 多次调用验证返回值在枚举范围内
for i in $(seq 1 10); do
  STATUS=$(curl -s "http://localhost:3000/sandbox/api/v1/users/1" \
    -H "X-Sandbox-Mode: mock" -H "X-Sandbox-Session: ${SESSION}" | jq -r '.status')
  echo "Mock status: ${STATUS}"
done
```

**期望：** 每次返回 `active`、`inactive` 或 `pending` 之一

##### 8.4 Fixed Response 覆盖

```bash
# 在 session config 中配置 fixed_response
curl -s -X POST "http://localhost:3000/api/v1/projects/${PROJECT_ID}/sandbox-sessions" \
  -H "Content-Type: application/json" \
  -d '{
    "tenant_id": "qa-team-2",
    "mode": "mock",
    "config": {"fixed_response": {"custom_field": "custom_value", "code": 42}},
    "expires_at": "2026-03-23T00:00:00Z"
  }' | jq .
```

**期望：** 所有请求返回固定的 `{"custom_field": "custom_value", "code": 42}`

##### 8.5 无需真实后端即可联调

- 不启动任何 WireMock/SSH 服务
- 验证 Mock 模式下所有路由正常返回数据
- 验证响应延迟 < 10ms（无后端调用开销）

---

### 场景 9: Replay 模式全链路

**组件：** `ReplayLayer` + `Recorder`

##### 9.1 Proxy 录制 → 切换 Replay → 回放

```bash
# 步骤 1: 创建 Proxy 会话并录制
PROXY_SESSION=$(curl -s -X POST "http://localhost:3000/api/v1/projects/${PROJECT_ID}/sandbox-sessions" \
  -H "Content-Type: application/json" \
  -d '{
    "tenant_id": "qa-team-3",
    "mode": "proxy",
    "config": {},
    "expires_at": "2026-03-23T00:00:00Z"
  }' | jq -r '.id')

# 发送请求录制交互
curl -s -X POST "http://localhost:3000/sandbox/api/v1/orders" \
  -H "X-Sandbox-Mode: proxy" \
  -H "X-Sandbox-Session: ${PROXY_SESSION}" \
  -H "Content-Type: application/json" \
  -d '{"customer_id": "C001", "items": [{"product_id": "P001"}]}' | jq .

# 步骤 2: 创建 Replay 会话
REPLAY_SESSION=$(curl -s -X POST "http://localhost:3000/api/v1/projects/${PROJECT_ID}/sandbox-sessions" \
  -H "Content-Type: application/json" \
  -d '{
    "tenant_id": "qa-team-3",
    "mode": "replay",
    "config": {},
    "expires_at": "2026-03-23T00:00:00Z"
  }' | jq -r '.id')

# 步骤 3: 用相同请求回放
curl -s -X POST "http://localhost:3000/sandbox/api/v1/orders" \
  -H "X-Sandbox-Mode: replay" \
  -H "X-Sandbox-Session: ${REPLAY_SESSION}" \
  -H "Content-Type: application/json" \
  -d '{"customer_id": "C001", "items": [{"product_id": "P001"}]}' | jq .
```

**期望：** Replay 返回与 Proxy 录制完全一致的响应

##### 9.2 精确匹配

- 请求 URL + Body 完全一致 → 返回录制的响应
- 任何字段差异 → 无法精确匹配

##### 9.3 模糊匹配（忽略时间戳）

- 请求体中时间戳字段不同但业务键一致 → 仍能匹配

##### 9.4 无匹配返回 404 + 最相似提示

```bash
# 发送未录制的请求
curl -s -X POST "http://localhost:3000/sandbox/api/v1/orders" \
  -H "X-Sandbox-Mode: replay" \
  -H "X-Sandbox-Session: ${REPLAY_SESSION}" \
  -H "Content-Type: application/json" \
  -d '{"customer_id": "C999", "items": []}' | jq .
```

**期望输出：**

```json
{
  "type": "https://api-anything.dev/errors/not-found",
  "title": "Not Found",
  "status": 404,
  "detail": "No matching recorded interaction found"
}
```

---

### 场景 10: Proxy 模式全链路

**组件：** `ProxyLayer` + `Recorder`

##### 10.1 透传到真实后端

```bash
curl -s -X POST "http://localhost:3000/sandbox/api/v1/orders" \
  -H "X-Sandbox-Mode: proxy" \
  -H "X-Sandbox-Session: ${PROXY_SESSION}" \
  -H "Content-Type: application/json" \
  -d '{"customer_id": "C001"}' | jq .
```

**验证：** 请求透传到真实后端，返回真实响应

##### 10.2 租户隔离验证 (X-Sandbox-Tenant)

```bash
# 验证后端收到的请求包含 X-Sandbox-Tenant header
curl -s http://localhost:9090/__admin/requests | \
  jq '.requests[0].headers["X-Sandbox-Tenant"]'
```

**期望：** 值为 session 创建时设置的 `tenant_id`

##### 10.3 数据染色验证

- 验证请求/响应自动标记 `_sandbox: true`
- 防止沙箱数据污染生产环境

##### 10.4 只读模式限制 (GET only)

```bash
# 创建 read_only 会话
RO_SESSION=$(curl -s -X POST "http://localhost:3000/api/v1/projects/${PROJECT_ID}/sandbox-sessions" \
  -H "Content-Type: application/json" \
  -d '{
    "tenant_id": "qa-team-ro",
    "mode": "proxy",
    "config": {"read_only": true},
    "expires_at": "2026-03-23T00:00:00Z"
  }' | jq -r '.id')

# GET 请求通过
GET_STATUS=$(curl -s -o /dev/null -w '%{http_code}' \
  "http://localhost:3000/sandbox/api/v1/orders" \
  -H "X-Sandbox-Mode: proxy" -H "X-Sandbox-Session: ${RO_SESSION}")
echo "GET: ${GET_STATUS}"  # 期望 200

# POST 请求被拒绝
POST_STATUS=$(curl -s -o /dev/null -w '%{http_code}' \
  -X POST "http://localhost:3000/sandbox/api/v1/orders" \
  -H "X-Sandbox-Mode: proxy" -H "X-Sandbox-Session: ${RO_SESSION}" \
  -H "Content-Type: application/json" -d '{}')
echo "POST: ${POST_STATUS}"  # 期望 400 (BadRequest)
```

**期望响应（POST 被拒绝）：**

```json
{
  "type": "https://api-anything.dev/errors/bad-request",
  "title": "Bad Request",
  "status": 400,
  "detail": "Sandbox session is read-only, only GET requests allowed"
}
```

##### 10.5 自动录制交互

```bash
# 通过 Proxy 发送请求后，验证 recorded_interactions 表有记录
psql -c "SELECT id, session_id, route_id, duration_ms FROM recorded_interactions WHERE session_id = '${PROXY_SESSION}'"
```

---

### 场景 11: 多模式协作

##### 11.1 Mock → 开发联调

1. 创建 Mock 会话
2. 前端团队使用 Mock 数据进行 UI 开发
3. 验证所有接口返回符合 Schema 的数据

##### 11.2 Replay → 回归测试

1. 从 Proxy 会话导出录制数据
2. 创建 Replay 会话
3. 运行完整回归测试套件
4. 对比所有响应与录制一致

##### 11.3 Proxy → 预发布验证

1. 创建 Proxy 会话指向预发布环境
2. 执行完整功能验证
3. 验证真实后端交互正确

##### 11.4 三种模式无缝切换

```bash
# 同一项目下依次创建三种会话
for MODE in mock replay proxy; do
  curl -s -X POST "http://localhost:3000/api/v1/projects/${PROJECT_ID}/sandbox-sessions" \
    -H "Content-Type: application/json" \
    -d "{\"tenant_id\": \"switch-test\", \"mode\": \"${MODE}\", \"config\": {}, \"expires_at\": \"2026-03-23T00:00:00Z\"}" | jq .mode
done
```

**验证：** 模式切换不影响已有会话，各会话独立运行

---

## 5. 补偿机制全链路测试

### 场景 12: at_least_once 全链路

**组件：** `RequestLogger` + `RetryWorker` + `RetryConfig`

##### 12.1 请求 → 记录 → 后端失败 → 自动重试 → 恢复 → 成功

```bash
# 步骤 1: 配置路由 delivery_guarantee = at_least_once
# 步骤 2: 让后端暂时不可用
curl -X POST http://localhost:9090/__admin/mappings -d '{
  "request": {"method": "POST", "url": "/orders"},
  "response": {"status": 500, "body": "Service Unavailable"}
}'

# 步骤 3: 发送请求（首次失败）
curl -s -X POST http://localhost:3000/gw/api/v1/orders \
  -H "Content-Type: application/json" \
  -d '{"customer_id": "C001", "product": "Widget"}' | jq .

# 步骤 4: 验证投递记录创建
psql -c "SELECT id, status, retry_count, next_retry_at FROM delivery_records ORDER BY created_at DESC LIMIT 1"
# 期望: status = 'failed', retry_count = 0

# 步骤 5: 等待第一次重试（1s 后）
sleep 2
psql -c "SELECT id, status, retry_count FROM delivery_records ORDER BY created_at DESC LIMIT 1"
# 期望: status = 'failed', retry_count = 1

# 步骤 6: 恢复后端
curl -X POST http://localhost:9090/__admin/mappings/reset
curl -X POST http://localhost:9090/__admin/mappings -d '{
  "request": {"method": "POST", "url": "/orders"},
  "response": {"status": 200, "body": "{\"order_id\": \"ORD-001\"}"}
}'

# 步骤 7: 等待下次重试成功（5s 后）
sleep 6
psql -c "SELECT id, status, retry_count FROM delivery_records ORDER BY created_at DESC LIMIT 1"
# 期望: status = 'delivered'
```

##### 12.2 验证指数退避时间

```bash
# 查询连续重试的 next_retry_at 间隔
psql -c "
  SELECT retry_count,
         next_retry_at,
         next_retry_at - LAG(next_retry_at) OVER (ORDER BY retry_count) AS delay
  FROM delivery_records
  WHERE route_id = '${ROUTE_ID}'
  ORDER BY retry_count
"
```

**期望延迟序列：**

| 重试次数 | 延迟 |
|---------|------|
| 第 1 次 | 1s |
| 第 2 次 | 5s |
| 第 3 次 | 30s |
| 第 4 次 | 5min |
| 第 5 次 | 30min |

##### 12.3 验证 delivery_records 状态流转

```
pending → failed (首次失败) → failed (重试失败) → ... → delivered (最终成功)
pending → failed → failed → ... → dead (超出最大重试次数)
```

---

### 场景 13: exactly_once 全链路

**组件：** `RequestLogger` + `IdempotencyGuard` + `RetryWorker`

##### 13.1 首次请求 → 创建幂等键 → 后端调用 → 标记 delivered

```bash
# 发送带幂等键的请求
curl -s -X POST http://localhost:3000/gw/api/v1/payments \
  -H "Content-Type: application/json" \
  -H "Idempotency-Key: pay-20260322-001" \
  -d '{"amount": 100.00, "currency": "CNY", "to": "merchant-001"}' | jq .
```

**期望：** HTTP 200，支付成功

```bash
# 验证幂等键状态
psql -c "SELECT * FROM idempotency_keys WHERE idempotency_key = 'pay-20260322-001'"
# 期望: status = 'delivered', response_hash 非空
```

##### 13.2 重复请求 → 返回 already_delivered

```bash
# 使用相同幂等键再次发送
curl -s -X POST http://localhost:3000/gw/api/v1/payments \
  -H "Content-Type: application/json" \
  -H "Idempotency-Key: pay-20260322-001" \
  -d '{"amount": 100.00, "currency": "CNY", "to": "merchant-001"}' | jq .
```

**期望：** 返回 AlreadyDelivered 错误（HTTP 409 或 200 + 缓存响应）

##### 13.3 并发重复请求 → 第二个被拒绝

```bash
# 同时发送两个相同幂等键的请求
curl -s -X POST http://localhost:3000/gw/api/v1/payments \
  -H "Idempotency-Key: pay-20260322-002" \
  -H "Content-Type: application/json" \
  -d '{"amount": 200.00}' &
PID1=$!

curl -s -X POST http://localhost:3000/gw/api/v1/payments \
  -H "Idempotency-Key: pay-20260322-002" \
  -H "Content-Type: application/json" \
  -d '{"amount": 200.00}' &
PID2=$!

wait $PID1 $PID2
```

**期望：** 一个返回 200，另一个返回 400（"Request is already being processed"）

##### 13.4 缺少 Idempotency-Key 头

```bash
# exactly_once 路由不带 Idempotency-Key
curl -s -X POST http://localhost:3000/gw/api/v1/payments \
  -H "Content-Type: application/json" \
  -d '{"amount": 50.00}' | jq .
```

**期望：**

```json
{
  "type": "https://api-anything.dev/errors/bad-request",
  "title": "Bad Request",
  "status": 400,
  "detail": "Idempotency-Key header required for exactly-once delivery"
}
```

##### 13.5 幂等键过期/清理

- 验证过期幂等键在 TTL 后被自动清理
- 清理后使用相同 key 发送请求应作为新请求处理

---

### 场景 14: 死信处理全链路

**组件：** `DeadLetterProcessor` + 管理 API

##### 14.1 5 次重试全部失败 → 进入死信

```bash
# 配置后端始终返回 500（max_retries=5）
# 等待所有重试完成（约 1+5+30+300+1800 = ~36 分钟，测试环境可缩短 delays）
# 测试环境建议将 delays 配置为 [1s, 1s, 1s, 1s, 1s]

psql -c "SELECT id, status, retry_count, error_message FROM delivery_records WHERE status = 'dead'"
```

**期望：** status = 'dead', retry_count = 5

##### 14.2 管理 API 查看死信详情

```bash
# 列出所有死信
curl -s http://localhost:3000/api/v1/compensation/dead-letters | jq .

# 查看单条投递记录详情
curl -s "http://localhost:3000/api/v1/compensation/delivery-records/${RECORD_ID}" | jq .
```

**期望输出：**

```json
{
  "id": "<uuid>",
  "route_id": "<uuid>",
  "trace_id": "...",
  "request_payload": {"customer_id": "C001", "product": "Widget"},
  "status": "dead",
  "retry_count": 5,
  "error_message": "Service Unavailable",
  "created_at": "...",
  "updated_at": "..."
}
```

##### 14.3 手动重推 → 回到重试队列

```bash
# 恢复后端正常
curl -X POST http://localhost:9090/__admin/mappings/reset

# 手动重推
curl -s -X POST "http://localhost:3000/api/v1/compensation/dead-letters/${RECORD_ID}/retry" | jq .

# 验证状态变为 failed（等待下次 worker 轮询处理）
psql -c "SELECT id, status, next_retry_at FROM delivery_records WHERE id = '${RECORD_ID}'"
# 期望: status = 'failed', next_retry_at ≈ now()
```

##### 14.4 批量重推

```bash
curl -s -X POST http://localhost:3000/api/v1/compensation/dead-letters/batch-retry \
  -H "Content-Type: application/json" \
  -d "{\"ids\": [\"${RECORD_ID_1}\", \"${RECORD_ID_2}\", \"${RECORD_ID_3}\"]}" | jq .
```

**期望输出：**

```json
{
  "retried_count": 3
}
```

##### 14.5 标记已处理

```bash
# 人工确认不需要重推（例如已通过其他渠道处理）
curl -s -X POST "http://localhost:3000/api/v1/compensation/dead-letters/${RECORD_ID}/resolve" | jq .

# 验证状态
psql -c "SELECT status, error_message FROM delivery_records WHERE id = '${RECORD_ID}'"
# 期望: status = 'delivered', error_message = 'Manually resolved'
```

##### 14.6 非死信状态不允许重推

```bash
# 对 status != 'dead' 的记录尝试重推
curl -s -X POST "http://localhost:3000/api/v1/compensation/dead-letters/${PENDING_RECORD_ID}/retry" | jq .
```

**期望：**

```json
{
  "type": "https://api-anything.dev/errors/bad-request",
  "title": "Bad Request",
  "status": 400,
  "detail": "Record is not in dead letter state"
}
```

---

## 6. 文档服务测试

### 场景 15: OpenAPI 文档

##### 15.1 文档端点可用性

```bash
# 获取 OpenAPI JSON
curl -s http://localhost:3000/api/v1/docs/openapi.json | jq '.openapi, .info.title, (.paths | keys | length)'
```

**期望：**

```
"3.0.3"
"API-Anything Platform"
<路由数量>
```

##### 15.2 文档包含所有活跃路由

```bash
# 验证新增路由后文档自动更新
ROUTE_COUNT_BEFORE=$(curl -s http://localhost:3000/api/v1/docs/openapi.json | jq '.paths | keys | length')

# 创建新项目并生成路由（此处省略具体步骤）

ROUTE_COUNT_AFTER=$(curl -s http://localhost:3000/api/v1/docs/openapi.json | jq '.paths | keys | length')
echo "Before: ${ROUTE_COUNT_BEFORE}, After: ${ROUTE_COUNT_AFTER}"
# 期望: After > Before
```

##### 15.3 Swagger UI 可在线交互调试

```bash
# 验证 Swagger UI 页面可访问
STATUS=$(curl -s -o /dev/null -w '%{http_code}' http://localhost:3000/api/v1/docs)
echo "Swagger UI: ${STATUS}"  # 期望 200
```

---

### 场景 16: Agent Prompt

##### 16.1 Agent Prompt 包含完整操作描述

```bash
curl -s http://localhost:3000/api/v1/docs/agent-prompt | head -50
```

**验证：**

- 包含所有活跃路由的描述
- 包含请求/响应格式说明
- 格式适合直接嵌入 LLM 提示词

##### 16.2 新增路由后自动更新

- 添加新路由后验证 agent-prompt 内容包含新路由信息

---

## 7. 前端 Web 全链路测试

### 场景 17: 项目管理流程

##### 17.1 创建项目 → 查看列表 → 查看详情 → 删除项目

```bash
# 创建
PROJECT_ID=$(curl -s -X POST http://localhost:3000/api/v1/projects \
  -H "Content-Type: application/json" \
  -d '{"name":"e2e-test-project","description":"E2E Test","owner":"qa","source_type":"cli","source_config":{}}' \
  | jq -r '.id')

# 列表
curl -s http://localhost:3000/api/v1/projects | jq '.[].name' | grep -q 'e2e-test-project' && echo "PASS: 项目出现在列表中"

# 详情
curl -s "http://localhost:3000/api/v1/projects/${PROJECT_ID}" | jq .name
# 期望: "e2e-test-project"

# 删除
DELETE_STATUS=$(curl -s -o /dev/null -w '%{http_code}' -X DELETE "http://localhost:3000/api/v1/projects/${PROJECT_ID}")
echo "Delete: ${DELETE_STATUS}"  # 期望 200 或 204

# 验证已删除
GET_AFTER=$(curl -s -o /dev/null -w '%{http_code}' "http://localhost:3000/api/v1/projects/${PROJECT_ID}")
echo "After delete: ${GET_AFTER}"  # 期望 404
```

##### 17.2 表单验证

- 空名称 → 返回 400 错误
- 重复名称 → 返回 409 冲突
- 无效 source_type → 返回 422 校验错误

##### 17.3 不同协议类型项目

```bash
for TYPE in wsdl cli ssh pty; do
  curl -s -X POST http://localhost:3000/api/v1/projects \
    -H "Content-Type: application/json" \
    -d "{\"name\": \"test-${TYPE}\", \"description\": \"${TYPE} test\", \"owner\": \"qa\", \"source_type\": \"${TYPE}\", \"source_config\": {}}" \
    | jq '.source_type'
done
```

---

### 场景 18: 沙箱管理流程

##### 18.1 完整沙箱操作流程

```bash
# 创建 Mock 会话
SESSION_ID=$(curl -s -X POST "http://localhost:3000/api/v1/projects/${PROJECT_ID}/sandbox-sessions" \
  -H "Content-Type: application/json" \
  -d '{"tenant_id":"qa-1","mode":"mock","config":{},"expires_at":"2026-03-23T00:00:00Z"}' \
  | jq -r '.id')

# 列出会话
curl -s "http://localhost:3000/api/v1/projects/${PROJECT_ID}/sandbox-sessions" | jq '.[].mode'

# 使用会话进行测试
curl -s "http://localhost:3000/sandbox/api/v1/test" \
  -H "X-Sandbox-Mode: mock" -H "X-Sandbox-Session: ${SESSION_ID}" | jq .

# 删除会话
curl -s -X DELETE "http://localhost:3000/api/v1/sandbox-sessions/${SESSION_ID}"
```

##### 18.2 三种模式切换

- 在同一项目下创建 mock、replay、proxy 三种会话
- 验证各自独立运行

##### 18.3 过期会话处理

```bash
# 创建即将过期的会话
curl -s -X POST "http://localhost:3000/api/v1/projects/${PROJECT_ID}/sandbox-sessions" \
  -H "Content-Type: application/json" \
  -d '{"tenant_id":"qa-expire","mode":"mock","config":{},"expires_at":"2026-03-22T00:00:01Z"}'

# 等待过期后使用
sleep 2
STATUS=$(curl -s -o /dev/null -w '%{http_code}' \
  "http://localhost:3000/sandbox/api/v1/test" \
  -H "X-Sandbox-Mode: mock" -H "X-Sandbox-Session: ${EXPIRED_SESSION}")
echo "Expired session: ${STATUS}"  # 期望 410 Gone 或 404
```

---

### 场景 19: 补偿管理流程

##### 19.1 查看死信列表 → 批量重推 → 刷新验证

```bash
# 查看死信列表
DEAD_LETTERS=$(curl -s http://localhost:3000/api/v1/compensation/dead-letters)
echo "${DEAD_LETTERS}" | jq 'length'

# 提取前 3 条 ID 进行批量重推
IDS=$(echo "${DEAD_LETTERS}" | jq -r '.[0:3] | .[].id')
curl -s -X POST http://localhost:3000/api/v1/compensation/dead-letters/batch-retry \
  -H "Content-Type: application/json" \
  -d "{\"ids\": $(echo "${DEAD_LETTERS}" | jq '.[0:3] | [.[].id]')}" | jq .

# 验证状态变更
curl -s http://localhost:3000/api/v1/compensation/dead-letters | jq 'length'
# 期望: 数量减少 3
```

##### 19.2 单条重推与标记已处理

```bash
RECORD_ID=$(curl -s http://localhost:3000/api/v1/compensation/dead-letters | jq -r '.[0].id')

# 单条重推
curl -s -X POST "http://localhost:3000/api/v1/compensation/dead-letters/${RECORD_ID}/retry" | jq .

# 标记另一条为已处理
RECORD_ID2=$(curl -s http://localhost:3000/api/v1/compensation/dead-letters | jq -r '.[0].id')
curl -s -X POST "http://localhost:3000/api/v1/compensation/dead-letters/${RECORD_ID2}/resolve" | jq .
```

---

### 场景 20: API 文档流程

##### 20.1 查看 Swagger UI → 在线调试 API

- 访问 `http://localhost:3000/api/v1/docs`
- 验证页面正常渲染
- 通过 Swagger UI "Try it out" 功能调用 API

##### 20.2 查看 Agent Prompt → 复制给 AI

```bash
curl -s http://localhost:3000/api/v1/docs/agent-prompt > /tmp/agent-prompt.txt
wc -l /tmp/agent-prompt.txt  # 验证内容非空
```

##### 20.3 下载 OpenAPI JSON

```bash
curl -s http://localhost:3000/api/v1/docs/openapi.json -o /tmp/openapi.json
python3 -c "import json; json.load(open('/tmp/openapi.json'))" && echo "PASS: Valid JSON"
```

---

## 8. 性能和稳定性测试

### 场景 21: 高并发压测

##### 21.1 使用 wrk 对网关发起 10K QPS

```bash
# 基准压测（SOAP 路由）
wrk -t12 -c400 -d30s -s post.lua http://localhost:3000/gw/api/v1/calculator/add

# post.lua 内容:
# wrk.method = "POST"
# wrk.body   = '{"a": 1, "b": 2}'
# wrk.headers["Content-Type"] = "application/json"
```

##### 21.2 使用 k6 进行渐进式负载测试

```bash
k6 run - <<'EOF'
import http from 'k6/http';
import { check, sleep } from 'k6';

export const options = {
  stages: [
    { duration: '1m', target: 100 },   // 升压到 100 VU
    { duration: '3m', target: 1000 },   // 升压到 1000 VU
    { duration: '5m', target: 5000 },   // 升压到 5000 VU
    { duration: '3m', target: 10000 },  // 峰值 10000 VU
    { duration: '2m', target: 0 },      // 降压
  ],
  thresholds: {
    http_req_duration: ['p(99)<5000'],  // P99 < 5s
    http_req_failed: ['rate<0.01'],     // 错误率 < 1%
  },
};

export default function () {
  const res = http.post('http://localhost:3000/gw/api/v1/calculator/add',
    JSON.stringify({ a: Math.floor(Math.random() * 100), b: Math.floor(Math.random() * 100) }),
    { headers: { 'Content-Type': 'application/json' } }
  );
  check(res, {
    'status is 200 or 429': (r) => r.status === 200 || r.status === 429,
  });
  sleep(0.01);
}
EOF
```

##### 21.3 验证限流生效

- 在压测日志中确认 429 响应出现
- 429 比例应与配置的 rate_limit 一致

##### 21.4 验证延迟 P99 < 5s

- 从 k6 输出中提取 `http_req_duration` 的 P99 值
- 阈值：P99 < 5000ms

---

### 场景 22: 长时间稳定性

##### 22.1 持续 1 小时运行混合负载

```bash
k6 run - <<'EOF'
import http from 'k6/http';
import { check, sleep } from 'k6';

export const options = {
  vus: 200,
  duration: '1h',
  thresholds: {
    http_req_duration: ['p(99)<5000'],
    http_req_failed: ['rate<0.05'],
  },
};

const routes = [
  '/gw/api/v1/calculator/add',
  '/gw/api/v1/tools/echo',
  '/gw/api/v1/network/interfaces/status',
];

export default function () {
  const route = routes[Math.floor(Math.random() * routes.length)];
  const res = http.post(`http://localhost:3000${route}`,
    JSON.stringify({ a: 1, b: 2, message: 'test' }),
    { headers: { 'Content-Type': 'application/json' } }
  );
  check(res, { 'is ok': (r) => r.status < 500 });
  sleep(0.05);
}
EOF
```

##### 22.2 验证内存不泄漏

```bash
# 每 5 分钟采集一次进程内存
for i in $(seq 1 12); do
  RSS=$(ps -o rss= -p $(pgrep api-anything) | head -1)
  echo "$(date +%H:%M) RSS: ${RSS}KB"
  sleep 300
done
```

**验证：** RSS 不应持续增长超过初始值的 20%

##### 22.3 验证连接池正常

```bash
# 查看 PostgreSQL 连接数
psql -c "SELECT count(*) FROM pg_stat_activity WHERE application_name LIKE '%api-anything%'"
# 期望: 稳定在连接池配置范围内，无连接泄漏
```

---

### 场景 23: 故障恢复

##### 23.1 PostgreSQL 重启 → 服务自动重连

```bash
# 重启 PostgreSQL
docker restart api-anything-postgres

# 等待 5 秒后验证服务可用
sleep 5
STATUS=$(curl -s -o /dev/null -w '%{http_code}' http://localhost:3000/health/ready)
echo "After PG restart: ${STATUS}"  # 期望 200

# 验证路由仍可正常调用
curl -s http://localhost:3000/gw/api/v1/calculator/add \
  -X POST -H "Content-Type: application/json" -d '{"a":1,"b":2}' | jq .
```

##### 23.2 后端服务抖动 → 熔断 → 恢复

```bash
# 阶段 1: 后端持续返回 500，触发熔断
for i in $(seq 1 30); do
  curl -s -o /dev/null http://localhost:3000/gw/api/v1/fragile-endpoint -X POST -d '{}'
done

# 阶段 2: 验证熔断打开
STATUS=$(curl -s -o /dev/null -w '%{http_code}' http://localhost:3000/gw/api/v1/fragile-endpoint -X POST -d '{}')
echo "Circuit open: ${STATUS}"  # 期望 503

# 阶段 3: 恢复后端，等待 HalfOpen，验证自动恢复
sleep 10  # 等待 open_duration
curl -s http://localhost:3000/gw/api/v1/fragile-endpoint -X POST -d '{}' | jq .
# 期望: 200（HalfOpen 试探成功后关闭熔断器）
```

##### 23.3 Retry Worker 重启 → 继续处理未完成的重试

```bash
# 查看当前 pending 重试数
psql -c "SELECT count(*) FROM delivery_records WHERE status = 'failed' AND next_retry_at <= NOW()"

# 重启服务
docker restart api-anything-server

# 等待 worker 启动后验证 pending 记录被处理
sleep 10
psql -c "SELECT count(*) FROM delivery_records WHERE status = 'failed' AND next_retry_at <= NOW()"
# 期望: 数量减少或为 0
```

---

## 9. 安全测试

### 场景 24: 命令注入防护

##### 24.1 CLI 适配器

```bash
# 各种命令注入 payload
PAYLOADS=(
  '; rm -rf /'
  '$(whoami)'
  '`id`'
  '| cat /etc/passwd'
  '&& echo pwned'
  '\n rm -rf /'
  '${IFS}cat${IFS}/etc/passwd'
)

for PAYLOAD in "${PAYLOADS[@]}"; do
  RESULT=$(curl -s -X POST http://localhost:3000/gw/api/v1/tools/echo \
    -H "Content-Type: application/json" \
    -d "{\"message\": \"${PAYLOAD}\"}" | jq -r '.stdout // .detail')
  echo "Payload: [${PAYLOAD}] => [${RESULT}]"
done
```

**验证：** 每个 payload 被当作字面字符串传递给 echo，不会被 shell 解释执行。核心保障机制：`tokio::process::Command` 的 `.arg()` 方法绕过 shell 解释。

##### 24.2 SSH 适配器

```bash
# 命令模板参数注入
curl -s -X POST http://localhost:3000/gw/api/v1/network/interfaces \
  -H "Content-Type: application/json" \
  -d '{"interface": "eth0; cat /etc/shadow"}' | jq .
```

**验证：** 参数通过模板替换后作为 ssh 命令的一部分安全传递。需关注远端 shell 对参数的处理。

---

### 场景 25: 错误规范化

##### 25.1 所有错误响应符合 RFC 7807

```bash
# 收集各种错误类型的响应
ERRORS=(
  "http://localhost:3000/gw/api/v1/nonexistent"           # 404
  "http://localhost:3000/api/v1/projects/invalid-uuid"     # 400
)

for URL in "${ERRORS[@]}"; do
  echo "--- ${URL} ---"
  curl -s "${URL}" | jq '{type, title, status, detail}'
done
```

**验证每个错误响应包含：**

- `type`: URI 标识错误类型
- `title`: 人类可读的简短描述
- `status`: HTTP 状态码
- `detail`: 具体错误详情

##### 25.2 敏感信息不泄露到错误详情

```bash
# 验证 SSH 连接失败不泄露私钥路径
curl -s http://localhost:3000/gw/api/v1/network/auth-fail | jq .detail | grep -qi 'ssh_key\|private\|password'
echo "Sensitive info check: $?"  # 期望: 1 (grep 未匹配)

# 验证数据库错误不泄露 SQL 语句
curl -s http://localhost:3000/api/v1/projects/00000000-0000-0000-0000-000000000000 | jq .detail | grep -qi 'SELECT\|FROM\|WHERE\|postgres'
echo "SQL leak check: $?"  # 期望: 1 (grep 未匹配)
```

---

## 10. 测试执行报告模板

```markdown
# API-Anything E2E 测试执行报告

## 测试环境信息

| 项目 | 值 |
|------|------|
| 环境名称 | ___________________ |
| 服务版本 | ___________________ |
| 服务器 | ___________________ |
| PostgreSQL 版本 | ___________________ |
| Kafka 版本 | ___________________ |
| Rust 编译器版本 | ___________________ |
| 操作系统 | ___________________ |
| 测试工具版本 | curl:___ / k6:___ / wrk:___ / WireMock:___ |

## 执行信息

| 项目 | 值 |
|------|------|
| 执行日期 | ___________________ |
| 执行人 | ___________________ |
| 执行轮次 | 第 ___ 轮 |
| 总耗时 | ___________________ |

## 统计总览

| 状态 | 数量 | 占比 |
|------|------|------|
| 通过 (PASS) | ___ | ___% |
| 失败 (FAIL) | ___ | ___% |
| 阻塞 (BLOCKED) | ___ | ___% |
| 跳过 (SKIP) | ___ | ___% |
| **合计** | ___ | 100% |

## 各场景详细结果

| 场景编号 | 场景名称 | 子项 | 状态 | 备注 |
|---------|---------|------|------|------|
| 1 | SOAP 遗留系统全链路 | 基本流程 | ☐ PASS / ☐ FAIL | |
| 1.1 | | 多操作 WSDL | ☐ PASS / ☐ FAIL | |
| 1.2 | | 嵌套复杂类型 | ☐ PASS / ☐ FAIL | |
| 1.3 | | SOAP Fault 处理 | ☐ PASS / ☐ FAIL | |
| 1.4 | | WS-Security Header | ☐ PASS / ☐ FAIL | |
| 1.5 | | 大型 WSDL RAG 分块 | ☐ PASS / ☐ FAIL | |
| 2 | CLI 工具全链路 | 基本流程 | ☐ PASS / ☐ FAIL | |
| 2.1 | | 子命令嵌套 | ☐ PASS / ☐ FAIL | |
| 2.2 | | JSON 输出解析 | ☐ PASS / ☐ FAIL | |
| 2.3 | | 正则输出提取 | ☐ PASS / ☐ FAIL | |
| 2.4 | | 超时处理 | ☐ PASS / ☐ FAIL | |
| 2.5 | | 命令注入防护 | ☐ PASS / ☐ FAIL | |
| 2.6 | | 并发限制 | ☐ PASS / ☐ FAIL | |
| 3 | SSH 远程命令全链路 | 基本流程 | ☐ PASS / ☐ FAIL | |
| 3.1 | | 参数模板替换 | ☐ PASS / ☐ FAIL | |
| 3.2 | | 连接超时 | ☐ PASS / ☐ FAIL | |
| 3.3 | | 认证失败 | ☐ PASS / ☐ FAIL | |
| 3.4 | | 多跳 SSH | ☐ PASS / ☐ FAIL | |
| 4 | PTY 交互式会话全链路 | 基本流程 | ☐ PASS / ☐ FAIL | |
| 4.1 | | 多步骤交互 | ☐ PASS / ☐ FAIL | |
| 4.2 | | 提示符变化 | ☐ PASS / ☐ FAIL | |
| 4.3 | | 超时处理 | ☐ PASS / ☐ FAIL | |
| 5 | 限流保护 | 正常流量 | ☐ PASS / ☐ FAIL | |
| 5.2 | | 突发 429 | ☐ PASS / ☐ FAIL | |
| 5.3 | | 令牌恢复 | ☐ PASS / ☐ FAIL | |
| 5.4 | | 路由独立限流 | ☐ PASS / ☐ FAIL | |
| 6 | 熔断保护 | Closed→Open | ☐ PASS / ☐ FAIL | |
| 6.2 | | Open→HalfOpen | ☐ PASS / ☐ FAIL | |
| 6.3 | | HalfOpen→Closed | ☐ PASS / ☐ FAIL | |
| 6.4 | | HalfOpen→重新 Open | ☐ PASS / ☐ FAIL | |
| 6.5 | | 后端独立熔断 | ☐ PASS / ☐ FAIL | |
| 7 | 并发信号量 | CLI 10 并发 | ☐ PASS / ☐ FAIL | |
| 7.2 | | SSH 5 并发 | ☐ PASS / ☐ FAIL | |
| 7.3 | | PTY 3 并发 | ☐ PASS / ☐ FAIL | |
| 7.4 | | 超限排队/拒绝 | ☐ PASS / ☐ FAIL | |
| 8 | Mock 模式全链路 | 基本 Mock | ☐ PASS / ☐ FAIL | |
| 8.2 | | Smart Mock 字段 | ☐ PASS / ☐ FAIL | |
| 8.3 | | 枚举随机选择 | ☐ PASS / ☐ FAIL | |
| 8.4 | | Fixed Response | ☐ PASS / ☐ FAIL | |
| 8.5 | | 零后端联调 | ☐ PASS / ☐ FAIL | |
| 9 | Replay 模式全链路 | 录制→回放 | ☐ PASS / ☐ FAIL | |
| 9.2 | | 精确匹配 | ☐ PASS / ☐ FAIL | |
| 9.3 | | 模糊匹配 | ☐ PASS / ☐ FAIL | |
| 9.4 | | 无匹配 404 | ☐ PASS / ☐ FAIL | |
| 10 | Proxy 模式全链路 | 透传 | ☐ PASS / ☐ FAIL | |
| 10.2 | | 租户隔离 | ☐ PASS / ☐ FAIL | |
| 10.3 | | 数据染色 | ☐ PASS / ☐ FAIL | |
| 10.4 | | 只读模式 | ☐ PASS / ☐ FAIL | |
| 10.5 | | 自动录制 | ☐ PASS / ☐ FAIL | |
| 11 | 多模式协作 | 模式切换 | ☐ PASS / ☐ FAIL | |
| 12 | at_least_once 全链路 | 重试→恢复 | ☐ PASS / ☐ FAIL | |
| 12.2 | | 指数退避 | ☐ PASS / ☐ FAIL | |
| 12.3 | | 状态流转 | ☐ PASS / ☐ FAIL | |
| 13 | exactly_once 全链路 | 幂等首次 | ☐ PASS / ☐ FAIL | |
| 13.2 | | 重复拒绝 | ☐ PASS / ☐ FAIL | |
| 13.3 | | 并发幂等 | ☐ PASS / ☐ FAIL | |
| 13.4 | | 缺少 Key | ☐ PASS / ☐ FAIL | |
| 14 | 死信处理全链路 | 进入死信 | ☐ PASS / ☐ FAIL | |
| 14.2 | | 查看详情 | ☐ PASS / ☐ FAIL | |
| 14.3 | | 手动重推 | ☐ PASS / ☐ FAIL | |
| 14.4 | | 批量重推 | ☐ PASS / ☐ FAIL | |
| 14.5 | | 标记已处理 | ☐ PASS / ☐ FAIL | |
| 14.6 | | 非死信拒绝 | ☐ PASS / ☐ FAIL | |
| 15 | OpenAPI 文档 | 端点可用 | ☐ PASS / ☐ FAIL | |
| 15.2 | | 自动更新 | ☐ PASS / ☐ FAIL | |
| 15.3 | | Swagger UI | ☐ PASS / ☐ FAIL | |
| 16 | Agent Prompt | 内容完整 | ☐ PASS / ☐ FAIL | |
| 16.2 | | 自动更新 | ☐ PASS / ☐ FAIL | |
| 17 | 项目管理流程 | CRUD | ☐ PASS / ☐ FAIL | |
| 17.2 | | 表单验证 | ☐ PASS / ☐ FAIL | |
| 17.3 | | 多协议类型 | ☐ PASS / ☐ FAIL | |
| 18 | 沙箱管理流程 | 完整操作 | ☐ PASS / ☐ FAIL | |
| 18.2 | | 模式切换 | ☐ PASS / ☐ FAIL | |
| 18.3 | | 过期处理 | ☐ PASS / ☐ FAIL | |
| 19 | 补偿管理流程 | 死信管理 | ☐ PASS / ☐ FAIL | |
| 20 | API 文档流程 | Swagger/Prompt | ☐ PASS / ☐ FAIL | |
| 21 | 高并发压测 | 10K QPS | ☐ PASS / ☐ FAIL | |
| 22 | 长时间稳定性 | 1h 运行 | ☐ PASS / ☐ FAIL | |
| 23 | 故障恢复 | PG/后端/Worker | ☐ PASS / ☐ FAIL | |
| 24 | 命令注入防护 | CLI/SSH | ☐ PASS / ☐ FAIL | |
| 25 | 错误规范化 | RFC 7807 | ☐ PASS / ☐ FAIL | |

## 发现的缺陷列表

| 编号 | 严重级别 | 关联场景 | 缺陷描述 | 复现步骤 | 当前状态 |
|------|---------|---------|---------|---------|---------|
| BUG-001 | P0/P1/P2 | 场景 ___ | | | ☐ 新建 / ☐ 修复中 / ☐ 已修复 / ☐ 不修复 |
| BUG-002 | | | | | |
| BUG-003 | | | | | |

## 前端截图附录

（请在对应截图编号位置插入实际截图）

| 截图编号 | 预期内容描述 | 实际结果 |
|---------|------------|---------|
| SCR-001 | Dashboard 项目卡片列表 | ☐ 符合预期 / ☐ 有差异（说明: ） |
| SCR-002 | 创建项目弹窗表单 | ☐ 符合预期 / ☐ 有差异 |
| ... | ... | ... |
```

---

## 11. 前端测试截图清单

| 截图编号 | 页面 | 操作 | 预期显示内容 | 关键元素标注 |
|---------|------|------|------------|------------|
| SCR-001 | Dashboard | 初始加载 | 项目卡片列表，显示所有已创建项目 | 项目名称、协议类型标签（wsdl/cli/ssh/pty）、owner 信息、创建时间 |
| SCR-002 | Dashboard | 点击"创建项目" | 弹出模态表单 | 名称输入框、描述文本域、协议类型下拉选择、owner 输入框、提交按钮、取消按钮 |
| SCR-003 | Dashboard | 提交空表单 | 表单校验错误提示 | 名称字段下方红色错误文字"项目名称不能为空"、提交按钮禁用状态 |
| SCR-004 | Dashboard | 创建成功 | 成功提示通知 + 新项目出现在列表中 | 绿色 Toast 通知"项目创建成功"、新卡片高亮显示 |
| SCR-005 | 项目详情 | 点击项目卡片 | 项目详情页，含契约、路由、后端绑定信息 | 项目元信息区域、Contract 版本列表、Route 表格（method + path + status）、BackendBinding 配置摘要 |
| SCR-006 | 项目详情 | 点击"删除项目" | 确认删除弹窗 | 警告图标、"确定删除项目 xxx 吗？"文字、确认按钮（红色）、取消按钮 |
| SCR-007 | 沙箱管理 | 进入沙箱页面 | 当前项目的沙箱会话列表 | 会话 ID、模式标签（Mock/Replay/Proxy）、租户 ID、过期时间、状态指示灯 |
| SCR-008 | 沙箱管理 | 点击"创建沙箱会话" | 创建表单弹窗 | 模式单选（Mock/Replay/Proxy）、租户 ID 输入框、过期时间日期选择器、配置 JSON 编辑器 |
| SCR-009 | 沙箱管理 | 选择 Mock 模式创建 | 创建成功后显示 cURL 示例 | cURL 命令文本框（含 X-Sandbox-Mode 和 X-Sandbox-Session header）、一键复制按钮 |
| SCR-010 | 沙箱管理 | 点击"查看 cURL" | 展开 cURL 命令详情 | 完整 curl 命令、请求头说明、各模式切换 Tab |
| SCR-011 | 沙箱管理 | 点击沙箱会话行 | 展开会话详情，含录制的交互列表 | 交互记录表格（请求方法、路径、响应状态码、耗时、录制时间） |
| SCR-012 | 沙箱管理 | 点击"删除会话" | 确认删除弹窗 | 删除确认文字、关联数据清理提示 |
| SCR-013 | 补偿管理 | 初始加载 | 死信队列列表 | 记录 ID、关联路由、trace_id、重试次数、错误信息摘要、创建时间、操作按钮 |
| SCR-014 | 补偿管理 | 勾选多条记录 | 批量操作工具栏出现 | 全选复选框、已选计数、"批量重推"按钮、"批量标记已处理"按钮 |
| SCR-015 | 补偿管理 | 点击"批量重推" | 确认弹窗 + 执行后刷新列表 | 确认弹窗显示选中数量、执行后 Toast 通知"已重推 N 条记录"、列表刷新 |
| SCR-016 | 补偿管理 | 展开单条记录 | 请求 payload 详情面板 | JSON 格式化展示 request_payload、response_payload（如有）、完整 error_message、状态流转时间线 |
| SCR-017 | 补偿管理 | 点击"单条重推" | 操作确认 + 状态更新 | 状态从"dead"变为"failed"（等待 worker 处理）、操作按钮变灰 |
| SCR-018 | 补偿管理 | 点击"标记已处理" | 操作确认 + 状态更新 | 状态变为"delivered"、备注显示"Manually resolved"、该行样式变为灰色 |
| SCR-019 | API 文档 | 加载 Swagger UI | 完整的 OpenAPI 交互式文档 | API 分组列表、每个端点的请求/响应 Schema、"Try it out" 按钮、认证配置区域 |
| SCR-020 | API 文档 | 点击"Try it out" | 展开请求编辑器 | 请求体 JSON 编辑器（预填 Schema 示例）、参数输入框、"Execute" 按钮 |
| SCR-021 | API 文档 | 执行请求 | 显示实际响应 | 响应状态码、响应头、响应体（JSON 格式化）、cURL 命令复现 |
| SCR-022 | API 文档 | 查看 Agent Prompt | Agent Prompt 文本展示 | 完整 Prompt 文本、一键复制按钮、路由数量统计 |
| SCR-023 | API 文档 | 点击"下载 OpenAPI JSON" | 浏览器下载文件 | 文件名 openapi.json、文件大小提示 |
| SCR-024 | 健康检查 | 加载 /health 页面 | 系统健康状态摘要 | 服务状态（UP/DOWN）、数据库连接状态、Kafka 连接状态、最后检查时间 |
| SCR-025 | 全局导航 | 侧边栏展开 | 完整导航菜单 | Dashboard、项目管理、沙箱管理、补偿管理、API 文档、监控面板（外链 Grafana）六个导航项 |

---

## 12. Phase 6 全链路测试场景

### 场景 26: EventBus 全链路

**目标**: 验证事件总线在 PG 模式和 Kafka 模式下的事件发布与消费。

#### 26.1 PG 模式事件持久化

| 步骤 | 操作 | 预期结果 |
|------|------|---------|
| 1 | 确认 `EVENT_BUS_TYPE=pg`（默认值） | 服务启动日志显示 PG 事件总线初始化 |
| 2 | 触发一个事件（如创建项目或生成合约） | 事件成功发布 |
| 3 | 查询数据库事件表 | 事件记录已持久化，包含 event_type、payload、created_at |

#### 26.2 Kafka 模式事件发布（可选）

| 步骤 | 操作 | 预期结果 |
|------|------|---------|
| 1 | 设置 `EVENT_BUS_TYPE=kafka` 和 `KAFKA_BROKERS=localhost:9092` | 服务启动日志显示 Kafka 事件总线初始化 |
| 2 | 触发事件 | 事件发布到 Kafka topic |
| 3 | 使用 Kafka consumer 验证 | 消费者收到对应事件消息 |

### 场景 27: Webhook 推送全链路

**目标**: 验证从 Webhook 订阅创建到事件触发再到下游接收推送的完整链路。

| 步骤 | 操作 | 预期结果 |
|------|------|---------|
| 1 | 启动一个 HTTP 服务监听推送（如 `nc -l 9999` 或 mockbin） | 监听端口就绪 |
| 2 | `POST /api/v1/webhooks` 创建订阅，URL 指向监听服务，event_types 包含 `DeadLetter` | 201 创建成功 |
| 3 | 触发对应事件（如制造一条死信记录） | 事件产生 |
| 4 | 检查监听服务是否收到 HTTP POST 请求 | 收到推送，请求体包含 event_type 和 payload |
| 5 | `GET /api/v1/webhooks` 确认订阅存在 | 列表中包含已创建的订阅 |
| 6 | `DELETE /api/v1/webhooks/{id}` 删除订阅 | 204 删除成功；后续事件不再推送到该 URL |

### 场景 28: SDK 代码生成

**目标**: 验证 4 种语言 SDK 生成的正确性和代码结构完整性。

| 步骤 | 操作 | 预期结果 |
|------|------|---------|
| 1 | 确保数据库中存在已激活的路由（先执行一次 WSDL 生成） | 路由表非空 |
| 2 | `GET /api/v1/docs/sdk/typescript` | 200，响应体包含 `fetch`、`async`、`interface` 等 TypeScript 关键字 |
| 3 | `GET /api/v1/docs/sdk/python` | 200，响应体包含 `requests`、`class`、`def` 等 Python 关键字 |
| 4 | `GET /api/v1/docs/sdk/java` | 200，响应体包含 `HttpClient`、`public class` 等 Java 关键字 |
| 5 | `GET /api/v1/docs/sdk/go` | 200，响应体包含 `net/http`、`func`、`package` 等 Go 关键字 |
| 6 | `GET /api/v1/docs/sdk/cobol` | 400，错误响应提示不支持的语言 |

### 场景 29: Plugin 动态加载

**目标**: 验证插件从编译到加载到网关调用的完整链路。

| 步骤 | 操作 | 预期结果 |
|------|------|---------|
| 1 | 使用 `plugin-sdk` crate 编写一个测试插件，编译为 `.so` / `.dylib` | 编译成功，产出动态库文件 |
| 2 | 将动态库放入 `PLUGIN_DIR` 指定目录（默认 `./plugins`） | 文件就位 |
| 3 | `POST /api/v1/plugins/scan` 触发扫描 | 200，返回扫描结果，包含新发现的插件 |
| 4 | `GET /api/v1/plugins` 查看已加载插件 | 列表中包含刚加载的插件，显示名称和版本 |
| 5 | 通过网关调用该插件协议对应的路由 | 请求经过插件适配器处理，返回预期响应 |

### 场景 30: Contract 变更日志

**目标**: 验证合约版本之间的差异对比和 Breaking Change 检测。

| 步骤 | 操作 | 预期结果 |
|------|------|---------|
| 1 | 对同一项目执行两次生成（不同版本的 WSDL） | 产生两个合约版本 |
| 2 | 对比两个版本的合约差异 | 差异报告包含新增/删除/修改的路由和字段 |
| 3 | 引入 Breaking Change（如删除一个必填字段或移除路由） | 差异报告明确标记为 Breaking Change |
| 4 | 引入非 Breaking Change（如新增可选字段） | 差异报告标记为兼容变更 |
