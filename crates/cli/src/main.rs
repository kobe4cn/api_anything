use clap::Parser;
use api_anything_common::config::AppConfig;
use api_anything_common::models::SourceType;
use api_anything_metadata::{MetadataRepo, PgMetadataRepo};
use api_anything_generator::pipeline::GenerationPipeline;
use sqlx::PgPool;

#[derive(Parser)]
#[command(name = "api-anything", about = "AI-powered legacy system API gateway generator")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Generate REST API from a WSDL file
    Generate {
        /// Path to WSDL file
        #[arg(short, long)]
        source: String,

        /// Project name
        #[arg(short, long)]
        project: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Generate { source, project } => {
            let config = AppConfig::from_env();
            let pool = PgPool::connect(&config.database_url).await?;
            let repo = PgMetadataRepo::new(pool);
            // 确保迁移已执行，允许 CLI 作为独立入口在空库上直接运行
            repo.run_migrations().await?;

            let wsdl_content = std::fs::read_to_string(&source)
                .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", source, e))?;

            // 创建项目记录，owner 固定为 "cli" 表示由命令行工具创建
            let project_obj = repo.create_project(
                &project, "Auto-generated", "cli", SourceType::Wsdl,
            ).await?;

            // 执行完整生成流水线：解析 → 映射 → 持久化 → 生成 OpenAPI
            let result = GenerationPipeline::run_wsdl(&repo, project_obj.id, &wsdl_content).await?;

            println!("Generation complete!");
            println!("  Contract ID: {}", result.contract_id);
            println!("  Routes created: {}", result.routes_count);

            // 将 OpenAPI 规范写入 <source>.openapi.json，方便直接导入 Postman 或 Swagger UI
            let spec_path = format!("{}.openapi.json", source);
            std::fs::write(&spec_path, serde_json::to_string_pretty(&result.openapi_spec)?)?;
            println!("  OpenAPI spec: {}", spec_path);
        }
    }

    Ok(())
}
