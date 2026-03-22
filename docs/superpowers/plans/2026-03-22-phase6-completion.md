# Phase 6: 功能补完 Implementation Plan

> 补全设计规格中所有未实现功能

**决策确认：**
- Kafka：可选增强，无 Kafka 时降级为 PG 轮询
- Push Dispatcher：可配置事件订阅
- 告警：Slack/钉钉 Webhook
- .so 插件：支持用户自定义协议适配器
- SSH 连接池：russh 纯 Rust 实现

---

## Phase 6a: Kafka + Push Dispatcher + Webhook 告警

### Task 1: Kafka 可选事件总线
- 创建 `crates/event-bus/` crate
- EventBus trait（publish/subscribe）+ PgEventBus 实现（当前轮询）+ KafkaEventBus 实现（rdkafka）
- 配置切换：`EVENT_BUS=kafka` 或 `EVENT_BUS=pg`（默认）
- 事件类型：RouteUpdated, DeliveryFailed, DeliverySucceeded, DeadLetter, GenerationCompleted
- 集成到 compensation retry_worker（从 PG 轮询迁移到 EventBus 消费）

### Task 2: Push Dispatcher（Webhook 推送）
- 创建 `crates/push-dispatcher/` 或在 compensation crate 中添加
- Subscription 模型：下游注册 webhook URL + 订阅事件类型 + 过滤条件
- DB 表：webhook_subscriptions (id, url, event_types, filter, active, created_at)
- 推送逻辑：EventBus 收到事件 → 匹配订阅 → HTTP POST 到 webhook URL
- 推送失败 → 进入重试队列（复用 RetryWorker）
- 管理 API：CRUD webhook 订阅
- Web 前端：Webhook 管理页面

### Task 3: Slack/钉钉 Webhook 告警
- AlertManager：监听 P0/P1 事件（熔断打开、死信增长、5xx > 5%）
- Slack Webhook 通知（POST https://hooks.slack.com/...）
- 钉钉 Webhook 通知（POST https://oapi.dingtalk.com/robot/send?...）
- 配置：ALERT_WEBHOOK_URL + ALERT_WEBHOOK_TYPE=slack|dingtalk
- 告警模板：包含事件类型、路由信息、时间、详情

## Phase 6b: SSH 连接池 + .so 插件加载

### Task 4: SSH 连接池 (russh)
- 替换系统 ssh 二进制为 russh 纯 Rust SSH 客户端
- SSH 会话池：Arc<Mutex<Vec<Session>>>，按 host:port:user 分桶
- 连接复用：执行完命令后不关闭连接，返回池中
- 健康检查：定期 ping，移除死连接
- 认证方式：密码 / 私钥文件 / SSH Agent
- 更新 SshAdapter 使用 russh 而非 tokio::process::Command("ssh")

### Task 5: .so 动态插件加载
- 定义 Plugin C ABI 接口（cdylib）
- PluginManager：加载 .so → 提取 ProtocolAdapter impl → 注册到 DashMap
- 插件生命周期：load → health_check → register → hot_swap → unload
- 插件 SDK crate：提供宏和 trait 让第三方开发者创建插件
- RouteLoader 支持 protocol=plugin 类型
- 文档：如何开发自定义协议插件

## Phase 6c: 开发者体验增强

### Task 6: API Explorer（类 Postman 交互式调试器）
- Web 前端新页面 `/explorer`
- 基于 OpenAPI spec 自动构建请求表单
- 支持切换目标：生产网关 / 沙箱 Mock / 沙箱 Replay
- 请求/响应高亮展示
- 历史记录保存

### Task 7: SDK 代码生成
- 后端端点 `POST /api/v1/docs/sdk/{language}`
- 支持语言：TypeScript, Python, Java, Go
- 使用 openapi-generator-cli（Java）或 openapi-typescript（轻量）
- 缓存生成结果，OpenAPI 变更时失效
- Web 前端：SDK 下载按钮

### Task 8: Contract 变更日志
- Contract 版本 diff 引擎：比较两个版本的 parsed_model
- Breaking Change 检测：字段删除、类型变更、必填新增
- 自动生成变更摘要
- Web 前端：变更日志页面
- 管理 API：GET /api/v1/contracts/{id}/changelog

### Task 9: RAG 大型 WSDL 分块
- 检测 WSDL 大小 > 阈值（如 50KB 或 20+ portType）
- 按 `<wsdl:portType>` 切分为独立块
- 每块独立解析，最后合并
- 跨块类型引用检查和解析

## Phase 6d: 运维完善

### Task 10: Grafana 面板嵌入
- 创建预置 Grafana Dashboard JSON（网关 QPS/延迟/错误率、熔断器状态、连接池使用率）
- Web 前端新页面 `/monitoring`
- iframe 嵌入 Grafana Dashboard（Anonymous Auth 已配置）
- Docker Compose 自动加载 Dashboard 配置

### Task 11: Proxy 自动录制完善
- sandbox handler 的 proxy 模式补全录制逻辑
- 录制数据管理 API（列表、删除、导出）
- Web 前端：录制数据浏览器
