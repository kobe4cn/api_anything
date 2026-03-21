use crate::repo::MetadataRepo;
use api_anything_common::error::AppError;
use api_anything_common::models::*;
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
