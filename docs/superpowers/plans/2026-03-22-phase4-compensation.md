# Phase 4: 数据补偿引擎 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为生产环境构建失败自动恢复能力 — 请求日志记录、指数退避自动重试、幂等键保障精确一次语义、死信队列处理，以及管理 API 支持人工介入。

**Architecture:** 新建 `compensation` crate。Request Logger 作为网关中间件，对配置了 at_least_once / exactly_once 投递保障的路由记录请求快照到 delivery_records 表。Retry Worker 作为后台 tokio task 定期轮询失败记录并重试（复用 BackendDispatcher）。Idempotency Guard 通过 PostgreSQL idempotency_keys 表实现精确一次语义。超过最大重试次数的记录进入死信状态并触发告警。管理 API 提供死信浏览和手动重推功能。

**Tech Stack:** tokio (后台任务调度), sqlx (PostgreSQL 轮询), 已有 gateway + metadata crate

**Spec:** `docs/superpowers/specs/2026-03-21-api-anything-platform-design.md` §7

---

## File Structure

```
crates/compensation/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── request_logger.rs           # 请求快照记录
    ├── idempotency.rs              # 幂等键检查
    ├── retry_worker.rs             # 指数退避重试调度器
    ├── dead_letter.rs              # 死信处理
    └── config.rs                   # 重试策略配置
```

---

### Task 1: Compensation Crate + Request Logger + 幂等键

**Files:**
- Create: `crates/compensation/Cargo.toml` + src 文件
- Modify: `Cargo.toml` (workspace)
- Modify: `crates/metadata/src/repo.rs` — delivery record + idempotency CRUD
- Modify: `crates/metadata/src/pg.rs` — 实现

MetadataRepo 扩展：
```rust
// Delivery Records
async fn create_delivery_record(&self, route_id: Uuid, trace_id: &str, idempotency_key: Option<&str>, request_payload: &Value) -> Result<DeliveryRecord, AppError>;
async fn update_delivery_status(&self, id: Uuid, status: DeliveryStatus, error_message: Option<&str>, next_retry_at: Option<DateTime<Utc>>) -> Result<(), AppError>;
async fn list_pending_retries(&self, limit: i64) -> Result<Vec<DeliveryRecord>, AppError>;
async fn list_dead_letters(&self, route_id: Option<Uuid>, limit: i64, offset: i64) -> Result<Vec<DeliveryRecord>, AppError>;
async fn get_delivery_record(&self, id: Uuid) -> Result<DeliveryRecord, AppError>;

// Idempotency
async fn check_idempotency(&self, key: &str) -> Result<Option<IdempotencyRecord>, AppError>;
async fn create_idempotency_record(&self, key: &str, route_id: Uuid) -> Result<(), AppError>;
async fn mark_idempotency_delivered(&self, key: &str, response_hash: &str) -> Result<(), AppError>;
```

需要在 common/models.rs 添加：
```rust
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct IdempotencyRecord {
    pub idempotency_key: String,
    pub route_id: Uuid,
    pub status: String,
    pub response_hash: Option<String>,
    pub created_at: DateTime<Utc>,
}
```

**request_logger.rs**：
```rust
pub struct RequestLogger;
impl RequestLogger {
    /// 根据路由的 delivery_guarantee 决定是否记录
    pub async fn log_if_needed(
        repo: &impl MetadataRepo,
        route: &RouteWithBinding,
        trace_id: &str,
        idempotency_key: Option<&str>,
        request_payload: &Value,
    ) -> Result<Option<Uuid>, AppError> {
        match route.delivery_guarantee {
            DeliveryGuarantee::AtMostOnce => Ok(None), // 不记录
            DeliveryGuarantee::AtLeastOnce => {
                let record = repo.create_delivery_record(route.route_id, trace_id, None, request_payload).await?;
                Ok(Some(record.id))
            }
            DeliveryGuarantee::ExactlyOnce => {
                let key = idempotency_key.ok_or(AppError::BadRequest("Idempotency-Key header required for exactly-once delivery".into()))?;
                // 幂等检查
                if let Some(existing) = repo.check_idempotency(key).await? {
                    if existing.status == "delivered" {
                        return Err(AppError::AlreadyDelivered); // 返回缓存结果
                    }
                    // pending = 正在处理中
                    return Err(AppError::BadRequest("Request is already being processed".into()));
                }
                repo.create_idempotency_record(key, route.route_id).await?;
                let record = repo.create_delivery_record(route.route_id, trace_id, Some(key), request_payload).await?;
                Ok(Some(record.id))
            }
        }
    }
}
```

**idempotency.rs**：幂等检查和标记投递完成的辅助方法。

测试 (4)：
- at_most_once 不创建记录
- at_least_once 创建记录
- exactly_once 需要幂等键
- 重复幂等键拒绝处理

Commit: `feat(compensation): add compensation crate with request logger and idempotency guard`

---

### Task 2: 重试调度器 (Retry Worker)

**Files:**
- Create: `crates/compensation/src/retry_worker.rs`
- Create: `crates/compensation/src/config.rs`

**config.rs** — 重试策略：
```rust
pub struct RetryConfig {
    pub max_retries: u32,
    pub delays: Vec<Duration>, // [1s, 5s, 30s, 5min, 30min]
    pub poll_interval: Duration, // Worker 轮询间隔
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 5,
            delays: vec![
                Duration::from_secs(1),
                Duration::from_secs(5),
                Duration::from_secs(30),
                Duration::from_secs(300),
                Duration::from_secs(1800),
            ],
            poll_interval: Duration::from_secs(5),
        }
    }
}
```

**retry_worker.rs**：
```rust
pub struct RetryWorker { ... }

impl RetryWorker {
    /// 启动后台重试循环（作为 tokio::spawn 的长期任务）
    pub async fn run(&self) {
        loop {
            if let Err(e) = self.process_batch().await {
                tracing::error!(error = %e, "Retry worker batch failed");
            }
            tokio::time::sleep(self.config.poll_interval).await;
        }
    }

    /// 处理一批待重试的记录
    async fn process_batch(&self) -> Result<(), anyhow::Error> {
        let records = self.repo.list_pending_retries(100).await?;
        for record in records {
            self.retry_one(&record).await;
        }
        Ok(())
    }

    /// 重试单条记录
    async fn retry_one(&self, record: &DeliveryRecord) {
        // 1. 查找对应的 dispatcher
        // 2. 重建 GatewayRequest
        // 3. 执行 dispatch
        // 4. 成功 → 更新 status=delivered
        // 5. 失败 → 增加 retry_count，计算 next_retry_at
        // 6. 超过 max_retries → 更新 status=dead
    }
}
```

`list_pending_retries` SQL：
```sql
SELECT * FROM delivery_records
WHERE status = 'failed'
  AND next_retry_at IS NOT NULL
  AND next_retry_at <= NOW()
ORDER BY next_retry_at ASC
LIMIT $1
```

测试 (3)：
- 指数退避时间计算正确
- 超过最大重试次数转为 dead
- 成功重试更新 status

Commit: `feat(compensation): add retry worker with exponential backoff`

---

### Task 3: 死信处理 + 管理 API

**Files:**
- Create: `crates/compensation/src/dead_letter.rs`
- Create: `crates/platform-api/src/routes/compensation.rs`

**dead_letter.rs**：
```rust
pub struct DeadLetterProcessor;

impl DeadLetterProcessor {
    /// 手动重推一条死信
    pub async fn retry_dead_letter(repo, dispatchers, record_id) -> Result<(), AppError> {
        let record = repo.get_delivery_record(record_id).await?;
        if record.status != DeliveryStatus::Dead {
            return Err(AppError::BadRequest("Record is not in dead letter state"));
        }
        // 重置状态为 failed，设置 next_retry_at = now
        repo.update_delivery_status(record_id, DeliveryStatus::Failed, None, Some(Utc::now())).await?;
        Ok(())
    }

    /// 批量重推
    pub async fn retry_batch(repo, ids: &[Uuid]) -> Result<u32, AppError> { ... }

    /// 标记为已处理（人工确认不需要重推）
    pub async fn mark_resolved(repo, record_id) -> Result<(), AppError> {
        repo.update_delivery_status(record_id, DeliveryStatus::Delivered, Some("Manually resolved"), None).await
    }
}
```

**管理 API 端点：**
- `GET /api/v1/compensation/dead-letters?route_id=&limit=&offset=` — 查看死信列表
- `POST /api/v1/compensation/dead-letters/{id}/retry` — 单条重推
- `POST /api/v1/compensation/dead-letters/batch-retry` — 批量重推
- `POST /api/v1/compensation/dead-letters/{id}/resolve` — 标记已处理
- `GET /api/v1/compensation/delivery-records/{id}` — 查看单条记录详情

测试 (3)：
- 死信列表 API
- 单条重推 API
- 标记已处理 API

Commit: `feat(compensation): add dead letter processor and management API`

---

### Task 4: 集成到网关 + 后台 Worker 启动

**Files:**
- Modify: `crates/platform-api/src/routes/gateway.rs` — 注入 RequestLogger
- Modify: `crates/platform-api/src/main.rs` — 启动 RetryWorker 后台任务
- Modify: `crates/platform-api/src/state.rs` — 可能需要扩展

在 gateway_handler 中，dispatch 成功后更新 delivery record status=delivered，失败后更新 status=failed + next_retry_at。

在 main.rs 中，启动 RetryWorker：
```rust
let worker = RetryWorker::new(repo.clone(), dispatchers.clone(), RetryConfig::default());
tokio::spawn(async move { worker.run().await });
```

测试 (2)：
- 网关请求触发 delivery record 创建
- E2E: 后端失败 → delivery record 标记为 failed

Commit: `feat: integrate compensation engine into gateway and start retry worker`

---

## Summary

| Task | 组件 | 产出 |
|------|------|------|
| 1 | Request Logger + Idempotency | 请求快照 + 幂等检查 + 4 测试 |
| 2 | Retry Worker | 指数退避重试 + 后台轮询 + 3 测试 |
| 3 | Dead Letter + Management API | 死信处理 + 5 个管理端点 + 3 测试 |
| 4 | 集成 | 网关注入 + Worker 启动 + 2 测试 |

**Phase 4 验收标准：** at_least_once 路由失败后自动重试，exactly_once 路由通过幂等键防重复，超过最大重试进入死信队列，管理 API 支持查看/重推/标记已处理。
