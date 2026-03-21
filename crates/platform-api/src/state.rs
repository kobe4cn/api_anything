use api_anything_gateway::dispatcher::BackendDispatcher;
use api_anything_gateway::router::DynamicRouter;
use api_anything_metadata::PgMetadataRepo;
use dashmap::DashMap;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

// AppState 在 Axum 的 handler 间共享，Clone 只复制引用计数；
// BackendDispatcher 含 Box<dyn ProtocolAdapter>，不可 Clone，因此用 Arc 包装后存入 DashMap
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub repo: Arc<PgMetadataRepo>,
    pub router: Arc<DynamicRouter>,
    pub dispatchers: Arc<DashMap<Uuid, Arc<BackendDispatcher>>>,
}
