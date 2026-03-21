use api_anything_generator::pipeline::GenerationPipeline;
use api_anything_metadata::{MetadataRepo, PgMetadataRepo};
use api_anything_common::models::SourceType;
use sqlx::PgPool;
use uuid::Uuid;

async fn setup() -> (PgMetadataRepo, PgPool) {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://api_anything:api_anything@localhost:5432/api_anything".to_string());
    let pool = PgPool::connect(&database_url).await.expect("Failed to connect to DB");
    let repo = PgMetadataRepo::new(pool.clone());
    repo.run_migrations().await.expect("Failed to run migrations");
    (repo, pool)
}

fn sample_wsdl() -> &'static str {
    include_str!("fixtures/calculator.wsdl")
}

#[tokio::test]
async fn pipeline_creates_correct_number_of_routes() {
    let (repo, pool) = setup().await;
    let suffix = Uuid::new_v4();

    let project = repo
        .create_project(&format!("pipeline-test-{suffix}"), "test", "team", SourceType::Wsdl)
        .await
        .unwrap();

    let result = GenerationPipeline::run_wsdl(&repo, project.id, sample_wsdl())
        .await
        .unwrap();

    // calculator.wsdl 有 Add 和 GetHistory 两个操作
    assert_eq!(result.routes_count, 2);

    // 清理
    sqlx::query("DELETE FROM projects WHERE id = $1").bind(project.id).execute(&pool).await.unwrap();
}

#[tokio::test]
async fn pipeline_generates_valid_openapi_spec() {
    let (repo, pool) = setup().await;
    let suffix = Uuid::new_v4();

    let project = repo
        .create_project(&format!("pipeline-openapi-{suffix}"), "test", "team", SourceType::Wsdl)
        .await
        .unwrap();

    let result = GenerationPipeline::run_wsdl(&repo, project.id, sample_wsdl())
        .await
        .unwrap();

    // OpenAPI spec 基本结构验证
    assert_eq!(result.openapi_spec["openapi"], "3.0.3");
    let paths = result.openapi_spec["paths"].as_object().unwrap();
    assert_eq!(paths.len(), 2);

    // 每个 path 应有 post 方法
    for (_, path_item) in paths {
        assert!(path_item["post"].is_object(), "Each operation should map to POST");
        assert!(path_item["post"]["requestBody"].is_object(), "Should have requestBody");
        assert!(path_item["post"]["responses"]["200"].is_object(), "Should have 200 response");
    }

    // 清理
    sqlx::query("DELETE FROM projects WHERE id = $1").bind(project.id).execute(&pool).await.unwrap();
}

#[tokio::test]
async fn pipeline_persists_routes_to_database() {
    let (repo, pool) = setup().await;
    let suffix = Uuid::new_v4();

    let project = repo
        .create_project(&format!("pipeline-persist-{suffix}"), "test", "team", SourceType::Wsdl)
        .await
        .unwrap();

    let result = GenerationPipeline::run_wsdl(&repo, project.id, sample_wsdl())
        .await
        .unwrap();

    // 通过 list_active_routes_with_bindings 验证路由确实写入了数据库
    let routes = repo.list_active_routes_with_bindings().await.unwrap();
    let our_routes: Vec<_> = routes.iter()
        .filter(|r| r.contract_id == result.contract_id)
        .collect();

    assert_eq!(our_routes.len(), 2);

    // 验证路由关联了正确的协议
    for route in &our_routes {
        assert_eq!(route.protocol, api_anything_common::models::ProtocolType::Soap);
        assert!(route.path.starts_with("/api/v1/calculator/"));
    }

    // 清理
    sqlx::query("DELETE FROM projects WHERE id = $1").bind(project.id).execute(&pool).await.unwrap();
}

#[tokio::test]
async fn pipeline_produces_zero_routes_on_empty_wsdl() {
    // 解析器对无效输入的容错处理：返回空结构而非 panic
    // 这种行为是合理的 — 解析器提取它能识别的内容，跳过不认识的
    let (repo, pool) = setup().await;
    let suffix = Uuid::new_v4();

    let project = repo
        .create_project(&format!("pipeline-empty-{suffix}"), "test", "team", SourceType::Wsdl)
        .await
        .unwrap();

    let result = GenerationPipeline::run_wsdl(&repo, project.id, "<definitions/>").await.unwrap();
    assert_eq!(result.routes_count, 0, "Empty WSDL should produce zero routes");

    // 清理
    sqlx::query("DELETE FROM projects WHERE id = $1").bind(project.id).execute(&pool).await.unwrap();
}
