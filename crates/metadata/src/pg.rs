use crate::repo::MetadataRepo;
use api_anything_common::error::AppError;
use api_anything_common::models::*;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

pub struct PgMetadataRepo {
    pool: PgPool,
}

impl PgMetadataRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 在应用启动时执行，确保 schema 与代码版本同步后才接受请求
    pub async fn run_migrations(&self) -> Result<(), sqlx::Error> {
        sqlx::migrate!("src/migrations")
            .run(&self.pool)
            .await?;
        Ok(())
    }
}

impl MetadataRepo for PgMetadataRepo {
    async fn create_project(&self, name: &str, description: &str, owner: &str, source_type: SourceType) -> Result<Project, AppError> {
        // RETURNING 子句避免二次查询，同时获取数据库生成的 id、created_at 等字段
        let project = sqlx::query_as!(
            Project,
            r#"
            INSERT INTO projects (name, description, owner, source_type)
            VALUES ($1, $2, $3, $4)
            RETURNING id, name, description, owner,
                      source_type AS "source_type: SourceType",
                      source_config, created_at, updated_at
            "#,
            name, description, owner, source_type as SourceType,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(project)
    }

    async fn get_project(&self, id: Uuid) -> Result<Project, AppError> {
        // fetch_optional 区分"未找到"和数据库错误，避免把 404 误报为 500
        let project = sqlx::query_as!(
            Project,
            r#"
            SELECT id, name, description, owner,
                   source_type AS "source_type: SourceType",
                   source_config, created_at, updated_at
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
        // 按 created_at 降序排列，最新创建的项目优先展示
        let projects = sqlx::query_as!(
            Project,
            r#"
            SELECT id, name, description, owner,
                   source_type AS "source_type: SourceType",
                   source_config, created_at, updated_at
            FROM projects ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(projects)
    }

    async fn delete_project(&self, id: Uuid) -> Result<(), AppError> {
        // rows_affected == 0 意味着该 id 不存在，需明确返回 404 而非静默成功
        let result = sqlx::query!("DELETE FROM projects WHERE id = $1", id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::NotFound(format!("Project {id} not found")));
        }
        Ok(())
    }

    async fn create_contract(
        &self,
        project_id: Uuid,
        version: &str,
        original_schema: &str,
        parsed_model: &serde_json::Value,
    ) -> Result<Contract, AppError> {
        // 运行时查询而非宏，规避 sqlx 编译期对自定义枚举类型注解的校验限制
        let contract = sqlx::query_as::<_, Contract>(
            r#"
            INSERT INTO contracts (project_id, version, original_schema, parsed_model)
            VALUES ($1, $2, $3, $4)
            RETURNING id, project_id, version, status,
                      original_schema, parsed_model, created_at, updated_at
            "#,
        )
        .bind(project_id)
        .bind(version)
        .bind(original_schema)
        .bind(parsed_model)
        .fetch_one(&self.pool)
        .await?;
        Ok(contract)
    }

    async fn create_backend_binding(
        &self,
        protocol: ProtocolType,
        endpoint_config: &serde_json::Value,
        timeout_ms: i64,
    ) -> Result<BackendBinding, AppError> {
        // 将枚举转为字符串并通过 ::protocol_type 显式类型转换，
        // 避免 sqlx 运行时无法推断 $1 对应的自定义枚举类型
        let protocol_str = match protocol {
            ProtocolType::Soap => "soap",
            ProtocolType::Http => "http",
            ProtocolType::Cli => "cli",
            ProtocolType::Ssh => "ssh",
            ProtocolType::Pty => "pty",
        };
        let binding = sqlx::query_as::<_, BackendBinding>(
            r#"
            INSERT INTO backend_bindings (protocol, endpoint_config, timeout_ms)
            VALUES ($1::protocol_type, $2, $3)
            RETURNING id, protocol,
                      endpoint_config, connection_pool_config, circuit_breaker_config,
                      rate_limit_config, retry_config, timeout_ms, auth_mapping
            "#,
        )
        .bind(protocol_str)
        .bind(endpoint_config)
        .bind(timeout_ms)
        .fetch_one(&self.pool)
        .await?;
        Ok(binding)
    }

    async fn create_route(
        &self,
        contract_id: Uuid,
        method: HttpMethod,
        path: &str,
        request_schema: &serde_json::Value,
        response_schema: &serde_json::Value,
        transform_rules: &serde_json::Value,
        backend_binding_id: Uuid,
    ) -> Result<Route, AppError> {
        // 同上，枚举转字符串后通过 SQL 侧类型转换绑定，避免运行时类型推断失败
        let method_str = match method {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Patch => "PATCH",
            HttpMethod::Delete => "DELETE",
        };
        let route = sqlx::query_as::<_, Route>(
            r#"
            INSERT INTO routes (contract_id, method, path, request_schema, response_schema, transform_rules, backend_binding_id)
            VALUES ($1, $2::http_method, $3, $4, $5, $6, $7)
            RETURNING id, contract_id, method,
                      path, request_schema, response_schema, transform_rules,
                      backend_binding_id, delivery_guarantee,
                      enabled, created_at, updated_at
            "#,
        )
        .bind(contract_id)
        .bind(method_str)
        .bind(path)
        .bind(request_schema)
        .bind(response_schema)
        .bind(transform_rules)
        .bind(backend_binding_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(route)
    }

    async fn create_sandbox_session(
        &self,
        project_id: Uuid,
        tenant_id: &str,
        mode: SandboxMode,
        config: &Value,
        expires_at: DateTime<Utc>,
    ) -> Result<SandboxSession, AppError> {
        // 枚举转字符串后通过 SQL 侧 ::sandbox_mode 类型转换绑定，与 protocol_type 处理方式一致
        let mode_str = match mode {
            SandboxMode::Mock => "mock",
            SandboxMode::Replay => "replay",
            SandboxMode::Proxy => "proxy",
        };
        let session = sqlx::query_as::<_, SandboxSession>(
            r#"
            INSERT INTO sandbox_sessions (project_id, tenant_id, mode, config, expires_at)
            VALUES ($1, $2, $3::sandbox_mode, $4, $5)
            RETURNING id, project_id, tenant_id, mode, config, expires_at, created_at
            "#,
        )
        .bind(project_id)
        .bind(tenant_id)
        .bind(mode_str)
        .bind(config)
        .bind(expires_at)
        .fetch_one(&self.pool)
        .await?;
        Ok(session)
    }

    async fn get_sandbox_session(&self, id: Uuid) -> Result<SandboxSession, AppError> {
        let session = sqlx::query_as::<_, SandboxSession>(
            r#"
            SELECT id, project_id, tenant_id, mode, config, expires_at, created_at
            FROM sandbox_sessions WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("SandboxSession {id} not found")))?;
        Ok(session)
    }

    async fn list_sandbox_sessions(&self, project_id: Uuid) -> Result<Vec<SandboxSession>, AppError> {
        // 按 created_at 降序排列，最新会话优先展示；过期会话由后台任务清理而非此处过滤
        let sessions = sqlx::query_as::<_, SandboxSession>(
            r#"
            SELECT id, project_id, tenant_id, mode, config, expires_at, created_at
            FROM sandbox_sessions WHERE project_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(sessions)
    }

    async fn delete_sandbox_session(&self, id: Uuid) -> Result<(), AppError> {
        let result = sqlx::query!("DELETE FROM sandbox_sessions WHERE id = $1", id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::NotFound(format!("SandboxSession {id} not found")));
        }
        Ok(())
    }

    async fn record_interaction(
        &self,
        session_id: Uuid,
        route_id: Uuid,
        request: &Value,
        response: &Value,
        duration_ms: i32,
    ) -> Result<RecordedInteraction, AppError> {
        let interaction = sqlx::query_as::<_, RecordedInteraction>(
            r#"
            INSERT INTO recorded_interactions (session_id, route_id, request, response, duration_ms)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id, session_id, route_id, request, response, duration_ms, recorded_at
            "#,
        )
        .bind(session_id)
        .bind(route_id)
        .bind(request)
        .bind(response)
        .bind(duration_ms)
        .fetch_one(&self.pool)
        .await?;
        Ok(interaction)
    }

    async fn find_matching_interaction(
        &self,
        session_id: Uuid,
        route_id: Uuid,
        request: &Value,
    ) -> Result<Option<RecordedInteraction>, AppError> {
        // 精确匹配优先：请求 JSON 完全相等时直接返回，避免进入开销较高的模糊匹配
        let exact = sqlx::query_as::<_, RecordedInteraction>(
            r#"
            SELECT id, session_id, route_id, request, response, duration_ms, recorded_at
            FROM recorded_interactions
            WHERE session_id = $1 AND route_id = $2 AND request = $3
            LIMIT 1
            "#,
        )
        .bind(session_id)
        .bind(route_id)
        .bind(request)
        .fetch_optional(&self.pool)
        .await?;

        if exact.is_some() {
            return Ok(exact);
        }

        // 模糊匹配：当请求体字段有细微差异时，按共同顶层 key 数量选出最相似录音；
        // 相同 key 数量时取最新录制，保证回放优先使用更接近当前业务语义的录音
        let candidates = sqlx::query_as::<_, RecordedInteraction>(
            r#"
            SELECT id, session_id, route_id, request, response, duration_ms, recorded_at
            FROM recorded_interactions
            WHERE session_id = $1 AND route_id = $2
            ORDER BY recorded_at DESC
            "#,
        )
        .bind(session_id)
        .bind(route_id)
        .fetch_all(&self.pool)
        .await?;

        if candidates.is_empty() {
            return Ok(None);
        }

        let request_keys: std::collections::HashSet<&str> = request
            .as_object()
            .map(|o| o.keys().map(|k| k.as_str()).collect())
            .unwrap_or_default();

        let best = candidates.into_iter().max_by_key(|interaction| {
            // 计算录音请求与当前请求共同的顶层 key 数量作为相似度分数
            interaction
                .request
                .as_object()
                .map(|o| o.keys().filter(|k| request_keys.contains(k.as_str())).count())
                .unwrap_or(0)
        });

        Ok(best)
    }

    async fn list_recorded_interactions(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<RecordedInteraction>, AppError> {
        let interactions = sqlx::query_as::<_, RecordedInteraction>(
            r#"
            SELECT id, session_id, route_id, request, response, duration_ms, recorded_at
            FROM recorded_interactions
            WHERE session_id = $1
            ORDER BY recorded_at DESC
            "#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(interactions)
    }

    async fn create_delivery_record(
        &self,
        route_id: Uuid,
        trace_id: &str,
        idempotency_key: Option<&str>,
        request_payload: &Value,
    ) -> Result<DeliveryRecord, AppError> {
        // RETURNING * 避免二次查询；status/retry_count 使用数据库默认值，
        // 业务层不需要知道这些字段的初始值
        let record = sqlx::query_as::<_, DeliveryRecord>(
            r#"
            INSERT INTO delivery_records (route_id, trace_id, idempotency_key, request_payload)
            VALUES ($1, $2, $3, $4)
            RETURNING id, route_id, trace_id, idempotency_key, request_payload,
                      response_payload, status, retry_count, next_retry_at,
                      error_message, created_at, updated_at
            "#,
        )
        .bind(route_id)
        .bind(trace_id)
        .bind(idempotency_key)
        .bind(request_payload)
        .fetch_one(&self.pool)
        .await?;
        Ok(record)
    }

    async fn update_delivery_status(
        &self,
        id: Uuid,
        status: DeliveryStatus,
        error_message: Option<&str>,
        next_retry_at: Option<DateTime<Utc>>,
    ) -> Result<(), AppError> {
        // delivery_status 是 PG 自定义枚举，需通过字符串 + SQL 侧类型转换绑定，
        // 与 protocol_type / sandbox_mode 的处理方式一致
        let status_str = match status {
            DeliveryStatus::Pending => "pending",
            DeliveryStatus::Delivered => "delivered",
            DeliveryStatus::Failed => "failed",
            DeliveryStatus::Dead => "dead",
        };
        // failed 状态时递增 retry_count，其他状态（delivered/dead）不增计数，
        // 避免将最终状态的写入误计为重试次数
        sqlx::query(
            r#"
            UPDATE delivery_records
            SET status = $2::delivery_status,
                error_message = $3,
                next_retry_at = $4,
                retry_count = CASE WHEN $2 = 'failed' THEN retry_count + 1 ELSE retry_count END
            WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(status_str)
        .bind(error_message)
        .bind(next_retry_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_pending_retries(&self, limit: i64) -> Result<Vec<DeliveryRecord>, AppError> {
        // next_retry_at <= NOW() 确保只捞取到期的记录，指数退避由调用方写入
        let records = sqlx::query_as::<_, DeliveryRecord>(
            r#"
            SELECT id, route_id, trace_id, idempotency_key, request_payload,
                   response_payload, status, retry_count, next_retry_at,
                   error_message, created_at, updated_at
            FROM delivery_records
            WHERE status = 'failed' AND next_retry_at <= NOW()
            ORDER BY next_retry_at
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(records)
    }

    async fn list_dead_letters(
        &self,
        route_id: Option<Uuid>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<DeliveryRecord>, AppError> {
        // route_id 为 None 时返回所有死信（全局视图），提供 route_id 时过滤至单条路由；
        // 运行时查询比条件宏更简洁，两个分支结构完全相同只是 WHERE 不同
        let records = if let Some(rid) = route_id {
            sqlx::query_as::<_, DeliveryRecord>(
                r#"
                SELECT id, route_id, trace_id, idempotency_key, request_payload,
                       response_payload, status, retry_count, next_retry_at,
                       error_message, created_at, updated_at
                FROM delivery_records
                WHERE status = 'dead' AND route_id = $1
                ORDER BY updated_at DESC
                LIMIT $2 OFFSET $3
                "#,
            )
            .bind(rid)
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, DeliveryRecord>(
                r#"
                SELECT id, route_id, trace_id, idempotency_key, request_payload,
                       response_payload, status, retry_count, next_retry_at,
                       error_message, created_at, updated_at
                FROM delivery_records
                WHERE status = 'dead'
                ORDER BY updated_at DESC
                LIMIT $1 OFFSET $2
                "#,
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?
        };
        Ok(records)
    }

    async fn get_delivery_record(&self, id: Uuid) -> Result<DeliveryRecord, AppError> {
        let record = sqlx::query_as::<_, DeliveryRecord>(
            r#"
            SELECT id, route_id, trace_id, idempotency_key, request_payload,
                   response_payload, status, retry_count, next_retry_at,
                   error_message, created_at, updated_at
            FROM delivery_records WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("DeliveryRecord {id} not found")))?;
        Ok(record)
    }

    async fn check_idempotency(&self, key: &str) -> Result<Option<IdempotencyRecord>, AppError> {
        let record = sqlx::query_as::<_, IdempotencyRecord>(
            r#"
            SELECT idempotency_key, route_id, status, response_hash, created_at
            FROM idempotency_keys WHERE idempotency_key = $1
            "#,
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;
        Ok(record)
    }

    async fn create_idempotency_record(&self, key: &str, route_id: Uuid) -> Result<(), AppError> {
        // ON CONFLICT DO NOTHING 防止并发请求中第二个写入报错；
        // 幂等键冲突时业务层已在 check_idempotency 中处理，此处静默忽略即可
        sqlx::query(
            r#"
            INSERT INTO idempotency_keys (idempotency_key, route_id)
            VALUES ($1, $2)
            ON CONFLICT (idempotency_key) DO NOTHING
            "#,
        )
        .bind(key)
        .bind(route_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn mark_idempotency_delivered(
        &self,
        key: &str,
        response_hash: &str,
    ) -> Result<(), AppError> {
        // response_hash 存储响应摘要而非完整响应体，避免大响应占用 idempotency_keys 表空间
        sqlx::query(
            r#"
            UPDATE idempotency_keys
            SET status = 'delivered', response_hash = $2
            WHERE idempotency_key = $1
            "#,
        )
        .bind(key)
        .bind(response_hash)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn create_webhook_subscription(
        &self,
        url: &str,
        event_types: &Value,
        description: &str,
    ) -> Result<WebhookSubscription, AppError> {
        let sub = sqlx::query_as::<_, WebhookSubscription>(
            r#"
            INSERT INTO webhook_subscriptions (url, event_types, description)
            VALUES ($1, $2, $3)
            RETURNING id, url, event_types, filter, headers, active, description, created_at, updated_at
            "#,
        )
        .bind(url)
        .bind(event_types)
        .bind(description)
        .fetch_one(&self.pool)
        .await?;
        Ok(sub)
    }

    async fn list_webhook_subscriptions(&self) -> Result<Vec<WebhookSubscription>, AppError> {
        let subs = sqlx::query_as::<_, WebhookSubscription>(
            r#"
            SELECT id, url, event_types, filter, headers, active, description, created_at, updated_at
            FROM webhook_subscriptions
            ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(subs)
    }

    async fn delete_webhook_subscription(&self, id: Uuid) -> Result<(), AppError> {
        let result = sqlx::query("DELETE FROM webhook_subscriptions WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::NotFound(format!("WebhookSubscription {id} not found")));
        }
        Ok(())
    }

    async fn list_active_subscriptions_for_event(
        &self,
        event_type: &str,
    ) -> Result<Vec<WebhookSubscription>, AppError> {
        // 匹配两种情况：event_types 包含目标事件类型，或 event_types 为空数组（订阅全部事件）
        let subs = sqlx::query_as::<_, WebhookSubscription>(
            r#"
            SELECT id, url, event_types, filter, headers, active, description, created_at, updated_at
            FROM webhook_subscriptions
            WHERE active = true
              AND (event_types @> $1::jsonb OR event_types = '[]'::jsonb)
            "#,
        )
        .bind(serde_json::json!([event_type]))
        .fetch_all(&self.pool)
        .await?;
        Ok(subs)
    }

    async fn get_route(&self, id: Uuid) -> Result<Route, AppError> {
        // fetch_optional 区分"未找到"与数据库错误，明确返回 404 而非 500
        let route = sqlx::query_as::<_, Route>(
            r#"
            SELECT id, contract_id, method,
                   path, request_schema, response_schema, transform_rules,
                   backend_binding_id, delivery_guarantee,
                   enabled, created_at, updated_at
            FROM routes WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Route {id} not found")))?;
        Ok(route)
    }

    async fn list_active_routes_with_bindings(&self) -> Result<Vec<RouteWithBinding>, AppError> {
        // 使用运行时查询而非宏，避免编译时对 enum 联合类型注解的复杂依赖
        // JOIN 查询确保只返回已配置后端绑定的路由，孤立路由不会出现在路由表中
        let rows = sqlx::query_as::<_, RouteWithBinding>(
            r#"
            SELECT
                r.id as route_id, r.contract_id,
                r.method as method,
                r.path, r.request_schema, r.response_schema, r.transform_rules,
                r.delivery_guarantee as delivery_guarantee,
                bb.id as binding_id,
                bb.protocol as protocol,
                bb.endpoint_config, bb.connection_pool_config, bb.circuit_breaker_config,
                bb.rate_limit_config, bb.retry_config, bb.timeout_ms, bb.auth_mapping
            FROM routes r
            JOIN backend_bindings bb ON r.backend_binding_id = bb.id
            WHERE r.enabled = true
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}
