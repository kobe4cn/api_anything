#[allow(unused_imports)]
use api_anything_common::models::SourceType;
use api_anything_gateway::dispatcher::BackendDispatcher;
#[allow(unused_imports)]
use api_anything_gateway::loader::RouteLoader;
use api_anything_gateway::router::DynamicRouter;
#[allow(unused_imports)]
use api_anything_metadata::{MetadataRepo, PgMetadataRepo};
use api_anything_platform_api::build_app;
use api_anything_platform_api::state::AppState;
use axum_test::TestServer;
use dashmap::DashMap;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

/// 启动测试服务器时手动构造 AppState，与 main.rs 保持一致；
/// gateway 组件初始化为空表，测试可按需通过 RouteLoader 或直接插入填充
pub async fn test_server() -> TestServer {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://api_anything:api_anything@localhost:5432/api_anything".to_string());
    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test DB");
    let repo = Arc::new(PgMetadataRepo::new(pool.clone()));
    repo.run_migrations().await.expect("Failed to run migrations");

    let router = Arc::new(DynamicRouter::new());
    let dispatchers: Arc<DashMap<Uuid, Arc<BackendDispatcher>>> = Arc::new(DashMap::new());
    let state = AppState { db: pool, repo, router, dispatchers };

    let app = build_app(state);
    TestServer::new(app).unwrap()
}

/// 返回数据库连接池，供需要直接操作数据库的测试使用
pub async fn test_pool() -> PgPool {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://api_anything:api_anything@localhost:5432/api_anything".to_string());
    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test DB");
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.expect("Failed to run migrations");
    pool
}

/// 创建完整的 SOAP 测试环境：DB + WSDL 生成 + 路由加载 + TestServer。
/// 使用唯一的 service name 后缀避免并行测试间路径冲突；
/// 返回 (TestServer, PgPool, 项目 ID, 路径前缀) 四元组，便于测试用例构造请求和清理数据
#[allow(dead_code)]
pub async fn setup_full_env(wsdl: &str, service_name_suffix: &str) -> (TestServer, PgPool, Uuid, String) {
    let pool = test_pool().await;
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let suffix = Uuid::new_v4();
    let project_name = format!("e2e-{}-{}", service_name_suffix, &suffix.to_string()[..8]);

    // 替换 WSDL 中的 service name 和 endpoint 为唯一名称，
    // 确保并行测试各自拥有独立的路由路径不会冲突
    let unique_id = &suffix.to_string().replace('-', "")[..16];
    let unique_svc_name = format!("Svc{}Service", unique_id);
    let unique_wsdl = wsdl.replace("CalculatorService", &unique_svc_name);

    let project = repo
        .create_project(&project_name, "e2e test", "test", SourceType::Wsdl)
        .await
        .unwrap();

    api_anything_generator::pipeline::GenerationPipeline::run_wsdl(&repo, project.id, &unique_wsdl, None)
        .await
        .unwrap();

    let router = Arc::new(DynamicRouter::new());
    let dispatchers: Arc<DashMap<Uuid, Arc<BackendDispatcher>>> = Arc::new(DashMap::new());
    RouteLoader::load(&repo, &router, &dispatchers).await.unwrap();

    let state = AppState {
        db: pool.clone(),
        repo: Arc::new(PgMetadataRepo::new(pool.clone())),
        router,
        dispatchers,
    };
    let server = TestServer::new(build_app(state)).unwrap();

    // WsdlMapper 先 kebab-case 再去 "-service" 后缀，
    // 返回生成的路径前缀以便测试用例构造 URL
    let path_prefix = to_kebab_strip_service(&unique_svc_name);
    (server, pool, project.id, path_prefix)
}

/// 清理测试数据：删除名称匹配指定模式的所有项目（级联删除关联的 contracts → routes）
#[allow(dead_code)]
pub async fn cleanup_project(pool: &PgPool, project_name_pattern: &str) {
    sqlx::query("DELETE FROM projects WHERE name LIKE $1")
        .bind(format!("%{}%", project_name_pattern))
        .execute(pool)
        .await
        .unwrap();
}

/// 复现 WsdlMapper::to_kebab_case 的逻辑并去掉 "-service" 后缀；
/// 用于在测试中预测生成路由的路径，无需访问内部实现
#[allow(dead_code)]
fn to_kebab_strip_service(s: &str) -> String {
    let mut result = String::new();
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() && i > 0 {
            result.push('-');
        }
        result.push(ch.to_lowercase().next().unwrap());
    }
    result
        .strip_suffix("-service")
        .unwrap_or(&result)
        .to_string()
}
