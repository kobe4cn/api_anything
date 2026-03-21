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
