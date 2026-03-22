use api_anything_common::models::{SandboxMode, SourceType};
use api_anything_metadata::{MetadataRepo, PgMetadataRepo};
use api_anything_sandbox::replay_layer::ReplayLayer;
use api_anything_sandbox::recorder::Recorder;
use api_anything_gateway::types::GatewayResponse;
use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

/// 连接数据库并运行迁移，保证测试在干净的 schema 下执行
async fn setup() -> (PgMetadataRepo, PgPool) {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://api_anything:api_anything@localhost:5432/api_anything".to_string()
    });
    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to DB");
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.expect("Failed to run migrations");
    (repo, pool)
}

/// 创建 replay 测试所需的项目 + 会话 + 路由；
/// route 需要 contract + backend_binding 作为外键，所以依赖较深，但这是真实业务场景的缩影
async fn make_session_and_route(
    repo: &PgMetadataRepo,
) -> (api_anything_common::models::Project, api_anything_common::models::SandboxSession, api_anything_common::models::Route) {
    use api_anything_common::models::{HttpMethod, ProtocolType};

    let suffix = Uuid::new_v4();
    let project = repo
        .create_project(
            &format!("replay-test-project-{suffix}"),
            "replay test",
            "team",
            SourceType::Wsdl,
        )
        .await
        .expect("create project");

    let session = repo
        .create_sandbox_session(
            project.id,
            "tenant-replay",
            SandboxMode::Replay,
            &serde_json::json!({}),
            chrono::Utc::now() + chrono::Duration::hours(1),
        )
        .await
        .expect("create session");

    let contract = repo
        .create_contract(project.id, "1.0.0", "<wsdl/>", &serde_json::json!({}))
        .await
        .expect("create contract");

    let binding = repo
        .create_backend_binding(
            ProtocolType::Soap,
            &serde_json::json!({"url": "http://example.com/soap"}),
            30000,
        )
        .await
        .expect("create binding");

    let route = repo
        .create_route(
            contract.id,
            HttpMethod::Post,
            &format!("/api/replay-test-{suffix}"),
            &serde_json::json!({}),
            &serde_json::json!({}),
            &serde_json::json!({}),
            binding.id,
        )
        .await
        .expect("create route");

    (project, session, route)
}

#[tokio::test]
async fn record_then_replay_exact_match() {
    let (repo, pool) = setup().await;
    let (project, session, route) = make_session_and_route(&repo).await;

    let request_body = serde_json::json!({"operation": "GetOrder", "order_id": "123"});
    let gateway_response = GatewayResponse {
        status_code: 200,
        headers: HashMap::new(),
        body: serde_json::json!({"order_id": "123", "status": "shipped"}),
    };

    // 录制一次交互
    Recorder::record(&repo, session.id, route.id, &request_body, &gateway_response, 42)
        .await
        .expect("record should succeed");

    // 精确匹配相同请求体时，应返回录制的响应；
    // response 以完整 GatewayResponse 序列化存储，body 字段包含实际响应内容
    let replayed = ReplayLayer::replay(&repo, session.id, route.id, &request_body)
        .await
        .expect("replay should succeed");

    assert_eq!(replayed["body"]["order_id"], "123");
    assert_eq!(replayed["body"]["status"], "shipped");

    // 清理（ON DELETE CASCADE 会级联删除 sessions、recorded_interactions）
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project.id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn replay_returns_not_found_without_recordings() {
    let (repo, pool) = setup().await;
    let (project, session, route) = make_session_and_route(&repo).await;

    // 未录制任何交互，replay 应返回 NotFound 而非 panic 或返回空值
    let request_body = serde_json::json!({"operation": "GetOrder"});
    let result = ReplayLayer::replay(&repo, session.id, route.id, &request_body).await;

    assert!(result.is_err());
    assert!(matches!(
        result,
        Err(api_anything_common::error::AppError::NotFound(_))
    ));

    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project.id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn list_recorded_interactions_returns_correct_count() {
    let (repo, pool) = setup().await;
    let (project, session, route) = make_session_and_route(&repo).await;

    let response = GatewayResponse {
        status_code: 200,
        headers: HashMap::new(),
        body: serde_json::json!({"ok": true}),
    };

    // 录制 3 条不同请求的交互，验证 list 返回数量正确
    for i in 0..3 {
        let req = serde_json::json!({"index": i});
        Recorder::record(&repo, session.id, route.id, &req, &response, 10)
            .await
            .expect("record should succeed");
    }

    let interactions = repo
        .list_recorded_interactions(session.id)
        .await
        .expect("list should succeed");

    assert_eq!(interactions.len(), 3);

    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(project.id)
        .execute(&pool)
        .await
        .unwrap();
}
