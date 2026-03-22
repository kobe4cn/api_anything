use api_anything_common::error::AppError;
use api_anything_common::models::*;
use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

/// 所有子系统通过此 trait 访问元数据，隔离存储实现细节，便于测试时替换为内存实现
pub trait MetadataRepo: Send + Sync {
    async fn create_project(&self, name: &str, description: &str, owner: &str, source_type: SourceType) -> Result<Project, AppError>;
    async fn get_project(&self, id: Uuid) -> Result<Project, AppError>;
    async fn list_projects(&self) -> Result<Vec<Project>, AppError>;
    async fn delete_project(&self, id: Uuid) -> Result<(), AppError>;
    /// 加载所有已启用路由及其后端绑定，供网关启动时填充动态路由表
    async fn list_active_routes_with_bindings(&self) -> Result<Vec<RouteWithBinding>, AppError>;

    /// 按 id 查询单条路由，沙箱 mock 模式需要读取 response_schema 生成模拟数据
    async fn get_route(&self, id: Uuid) -> Result<Route, AppError>;

    /// 从解析后的合约创建持久化记录；parsed_model 保存中间表示，避免后续重复解析原始 schema
    async fn create_contract(
        &self,
        project_id: Uuid,
        version: &str,
        original_schema: &str,
        parsed_model: &serde_json::Value,
    ) -> Result<Contract, AppError>;

    /// 创建后端绑定；连接池、熔断、限流、重试等配置由数据库默认值填充，调用方只需提供核心参数
    async fn create_backend_binding(
        &self,
        protocol: ProtocolType,
        endpoint_config: &serde_json::Value,
        timeout_ms: i64,
    ) -> Result<BackendBinding, AppError>;

    /// 创建路由，将合约操作与后端绑定关联；request/response schema 保存以便网关做运行时校验
    async fn create_route(
        &self,
        contract_id: Uuid,
        method: HttpMethod,
        path: &str,
        request_schema: &serde_json::Value,
        response_schema: &serde_json::Value,
        transform_rules: &serde_json::Value,
        backend_binding_id: Uuid,
    ) -> Result<Route, AppError>;

    /// 创建沙箱会话；expires_at 由调用方计算，使业务层控制过期策略而非数据库层
    async fn create_sandbox_session(
        &self,
        project_id: Uuid,
        tenant_id: &str,
        mode: SandboxMode,
        config: &Value,
        expires_at: DateTime<Utc>,
    ) -> Result<SandboxSession, AppError>;

    async fn get_sandbox_session(&self, id: Uuid) -> Result<SandboxSession, AppError>;

    /// 按 project_id 过滤，仅返回属于该项目的会话，防止跨项目数据泄露
    async fn list_sandbox_sessions(&self, project_id: Uuid) -> Result<Vec<SandboxSession>, AppError>;

    async fn delete_sandbox_session(&self, id: Uuid) -> Result<(), AppError>;

    /// 将一次请求/响应交互写入 recorded_interactions 表，供 replay 模式重放使用
    async fn record_interaction(
        &self,
        session_id: Uuid,
        route_id: Uuid,
        request: &Value,
        response: &Value,
        duration_ms: i32,
    ) -> Result<RecordedInteraction, AppError>;

    /// 先尝试精确匹配请求 JSON，若无精确命中则按顶层 key 数量取最相似记录；
    /// 模糊回退确保录制时请求字段略有差异也能复用已有录音，减少用户手动维护成本
    async fn find_matching_interaction(
        &self,
        session_id: Uuid,
        route_id: Uuid,
        request: &Value,
    ) -> Result<Option<RecordedInteraction>, AppError>;

    /// 返回指定会话的所有录音，按录制时间倒序，供调试和审计用
    async fn list_recorded_interactions(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<RecordedInteraction>, AppError>;

    /// 清空指定会话的所有录制数据，用于重置测试状态或释放存储空间
    async fn delete_recorded_interactions(
        &self,
        session_id: Uuid,
    ) -> Result<u64, AppError>;

    // ── 投递记录（补偿系统基础） ──────────────────────────────────────────

    /// 创建投递记录，在请求实际分发前写入，确保即使后端失败也有可重试凭据
    async fn create_delivery_record(
        &self,
        route_id: Uuid,
        trace_id: &str,
        idempotency_key: Option<&str>,
        request_payload: &Value,
    ) -> Result<DeliveryRecord, AppError>;

    /// 更新投递状态；error_message 和 next_retry_at 在 failed 时由调用方计算，
    /// dead 状态时两者可为 None（不再重试）
    async fn update_delivery_status(
        &self,
        id: Uuid,
        status: DeliveryStatus,
        error_message: Option<&str>,
        next_retry_at: Option<DateTime<Utc>>,
    ) -> Result<(), AppError>;

    /// 查询 next_retry_at <= now() 的 failed 记录，供重试 worker 轮询使用；
    /// limit 防止单次捞取过多导致内存压力
    async fn list_pending_retries(&self, limit: i64) -> Result<Vec<DeliveryRecord>, AppError>;

    /// 查询 dead 状态记录，route_id 为 None 时返回全局死信；
    /// offset 分页确保管理 API 不会一次性返回大量记录
    async fn list_dead_letters(
        &self,
        route_id: Option<Uuid>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<DeliveryRecord>, AppError>;

    /// 按 id 查询单条投递记录，重试 worker 需要原始请求体重新发起投递
    async fn get_delivery_record(&self, id: Uuid) -> Result<DeliveryRecord, AppError>;

    // ── 幂等键（ExactlyOnce 保证） ────────────────────────────────────────

    /// 查询幂等键是否已存在；返回 None 表示首次请求，Some 表示重复请求
    async fn check_idempotency(&self, key: &str) -> Result<Option<IdempotencyRecord>, AppError>;

    /// 在处理开始前写入幂等键（status = 'pending'），防止并发重复请求同时穿透
    async fn create_idempotency_record(&self, key: &str, route_id: Uuid) -> Result<(), AppError>;

    /// 投递成功后将幂等键置为 delivered 并记录响应摘要，
    /// 后续重复请求可直接返回 200 而无需重新处理
    async fn mark_idempotency_delivered(
        &self,
        key: &str,
        response_hash: &str,
    ) -> Result<(), AppError>;

    // ── Webhook 订阅管理 ─────────────────────────────────────────────────

    /// 创建 Webhook 订阅；event_types 为 JSON 数组，指定订阅的事件类型列表
    async fn create_webhook_subscription(
        &self,
        url: &str,
        event_types: &Value,
        description: &str,
    ) -> Result<WebhookSubscription, AppError>;

    /// 列出全部 Webhook 订阅，包含已禁用的，供管理界面展示
    async fn list_webhook_subscriptions(&self) -> Result<Vec<WebhookSubscription>, AppError>;

    /// 按 id 删除 Webhook 订阅；不存在时返回 404
    async fn delete_webhook_subscription(&self, id: Uuid) -> Result<(), AppError>;

    /// 查询匹配特定事件类型的活跃订阅；
    /// 使用 JSONB @> 运算符检查 event_types 数组是否包含目标事件，
    /// 同时匹配 event_types 为空数组的订阅（表示订阅所有事件）
    async fn list_active_subscriptions_for_event(
        &self,
        event_type: &str,
    ) -> Result<Vec<WebhookSubscription>, AppError>;
}
