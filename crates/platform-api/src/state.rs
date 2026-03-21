use api_anything_metadata::PgMetadataRepo;
use sqlx::PgPool;
use std::sync::Arc;

// AppState 在 Arc 中共享，Clone 只复制引用计数，
// 而非深拷贝整个数据库连接池
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub repo: Arc<PgMetadataRepo>,
}
