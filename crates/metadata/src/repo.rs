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
}
