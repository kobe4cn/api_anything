use api_anything_common::error::AppError;
use api_anything_common::models::*;
use uuid::Uuid;

/// 所有子系统通过此 trait 访问元数据，隔离存储实现细节，便于测试时替换为内存实现
pub trait MetadataRepo: Send + Sync {
    async fn create_project(&self, name: &str, description: &str, owner: &str, source_type: SourceType) -> Result<Project, AppError>;
    async fn get_project(&self, id: Uuid) -> Result<Project, AppError>;
    async fn list_projects(&self) -> Result<Vec<Project>, AppError>;
    async fn delete_project(&self, id: Uuid) -> Result<(), AppError>;
    /// 加载所有已启用路由及其后端绑定，供网关启动时填充动态路由表
    async fn list_active_routes_with_bindings(&self) -> Result<Vec<RouteWithBinding>, AppError>;

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
}
