/// 验证 RouteLoader 将数据库中的路由加载到网关后，该路由可通过 /gw/* 正常匹配；
/// 由于测试环境没有真实 SOAP 后端，预期得到 BackendUnavailable 错误（502/503/504），
/// 而不是 404——404 表示路由未加载成功
use api_anything_common::models::{HttpMethod, ProtocolType, SourceType};
use api_anything_metadata::MetadataRepo;
use axum::http::StatusCode;
use serde_json::json;

mod common;

#[tokio::test]
async fn loaded_route_is_reachable_via_gw_prefix() {
    // 先获取连接池，通过 repo 创建测试数据
    let pool = common::test_pool().await;
    let repo = api_anything_metadata::PgMetadataRepo::new(pool.clone());

    // 使用 UUID 后缀防止测试间 name UNIQUE 冲突
    let suffix = uuid::Uuid::new_v4();
    let project_name = format!("loader-test-{suffix}");

    // 1. 创建项目 → 合约 → 后端绑定 → 路由，模拟正常的数据配置流程
    let project = repo
        .create_project(&project_name, "loader integration test", "team-test", SourceType::Wsdl)
        .await
        .expect("create project");

    let contract = repo
        .create_contract(
            project.id,
            "1.0",
            "<definitions/>",
            &json!({}),
        )
        .await
        .expect("create contract");

    // endpoint_url 指向本地不存在的地址，确保测试环境中后端一定不可达
    let binding = repo
        .create_backend_binding(
            ProtocolType::Soap,
            &json!({
                "url": "http://127.0.0.1:19999/nonexistent-soap",
                "soap_action": "TestAction",
                "operation_name": "TestOp",
                "namespace": "http://test.example.com"
            }),
            5000,
        )
        .await
        .expect("create backend binding");

    let route_path = format!("/loader-test/{suffix}/orders");
    let _route = repo
        .create_route(
            contract.id,
            HttpMethod::Post,
            &route_path,
            &json!({}),
            &json!({}),
            &json!({}),
            binding.id,
        )
        .await
        .expect("create route");

    // 2. 构建已加载路由的测试服务器
    let server = make_server_with_loader(pool).await;

    // 3. 对刚创建的路由发起请求
    let gw_path = format!("/gw{route_path}");
    let response = server.post(&gw_path).json(&json!({"test": true})).await;

    // 路由已加载：状态码必须不是 404；
    // 由于后端不可达，期望 502/503/504/408 等后端错误，而非路由未找到的 404
    let status = response.status_code();
    assert_ne!(
        status,
        StatusCode::NOT_FOUND,
        "Expected backend error (not 404), got {status} — route may not have been loaded"
    );
}

/// 构造一个已执行路由加载的测试服务器；
/// 与 common::test_server() 的区别在于额外调用了 RouteLoader::load
async fn make_server_with_loader(pool: sqlx::PgPool) -> axum_test::TestServer {
    use api_anything_gateway::dispatcher::BackendDispatcher;
    use api_anything_gateway::loader::RouteLoader;
    use api_anything_gateway::router::DynamicRouter;
    use api_anything_metadata::PgMetadataRepo;
    use api_anything_platform_api::build_app;
    use api_anything_platform_api::state::AppState;
    use dashmap::DashMap;
    use std::sync::Arc;
    use uuid::Uuid;

    let repo = Arc::new(PgMetadataRepo::new(pool.clone()));
    let router = Arc::new(DynamicRouter::new());
    let dispatchers: Arc<DashMap<Uuid, Arc<BackendDispatcher>>> = Arc::new(DashMap::new());

    RouteLoader::load(repo.as_ref(), &router, &dispatchers)
        .await
        .expect("RouteLoader::load");

    let state = AppState { db: pool, repo, router, dispatchers };
    let app = build_app(state);
    axum_test::TestServer::new(app).unwrap()
}
