use api_anything_common::config::{AppConfig, LlmConfig};
use api_anything_common::models::SourceType;
use api_anything_generator::llm;
use api_anything_generator::pipeline::GenerationPipeline;
use api_anything_metadata::{MetadataRepo, PgMetadataRepo};
use clap::Parser;
use sqlx::PgPool;

#[derive(Parser)]
#[command(
    name = "api-anything",
    about = "AI-powered legacy system API gateway generator"
)]
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

    /// Generate REST API from SSH interaction sample
    GenerateSsh {
        /// Path to SSH interaction sample file
        #[arg(long)]
        sample: String,

        /// Project name
        #[arg(short, long)]
        project: String,
    },

    /// Generate REST API from CLI tool help output
    GenerateCli {
        /// Path to main help output text file
        #[arg(long)]
        main_help: String,

        /// Subcommand help files in name:path format (can repeat)
        #[arg(long)]
        sub_help: Vec<String>,

        /// Project name
        #[arg(short, long)]
        project: String,

        /// Path to the CLI executable
        #[arg(long)]
        program: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    // LLM 配置在所有子命令间共享，有 key 时启用 LLM 增强，无 key 时自动降级
    let llm_config = LlmConfig::from_env();
    let llm_client = llm::create_llm_client(&llm_config);

    if llm_client.is_some() {
        tracing::info!(
            provider = %llm_config.provider,
            model = %llm_config.model,
            "LLM enhancement enabled"
        );
    }

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
            let project_obj = repo
                .create_project(&project, "Auto-generated", "cli", SourceType::Wsdl)
                .await?;

            // 执行完整生成流水线，有 LLM 时自动启用增强映射
            let result = GenerationPipeline::run_wsdl(
                &repo,
                project_obj.id,
                &wsdl_content,
                llm_client.as_deref(),
            )
            .await?;

            println!("Generation complete!");
            println!("  Contract ID: {}", result.contract_id);
            println!("  Routes created: {}", result.routes_count);

            // 将 OpenAPI 规范写入 <source>.openapi.json，方便直接导入 Postman 或 Swagger UI
            let spec_path = format!("{}.openapi.json", source);
            std::fs::write(
                &spec_path,
                serde_json::to_string_pretty(&result.openapi_spec)?,
            )?;
            println!("  OpenAPI spec: {}", spec_path);
        }

        Commands::GenerateSsh { sample, project } => {
            let config = AppConfig::from_env();
            let pool = PgPool::connect(&config.database_url).await?;
            let repo = PgMetadataRepo::new(pool);
            // 允许 CLI 在空库上作为独立入口运行，迁移幂等，多次执行无副作用
            repo.run_migrations().await?;

            let sample_text = std::fs::read_to_string(&sample)
                .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", sample, e))?;

            // 创建项目记录，source_type 为 Ssh 以区别于 WSDL/CLI 来源
            let project_obj = repo
                .create_project(&project, "SSH command wrapper", "cli", SourceType::Ssh)
                .await?;

            // 执行 SSH 生成流水线
            let result = GenerationPipeline::run_ssh(
                &repo,
                project_obj.id,
                &sample_text,
                llm_client.as_deref(),
            )
            .await?;

            println!("SSH Generation complete!");
            println!("  Contract ID: {}", result.contract_id);
            println!("  Routes created: {}", result.routes_count);

            // 将 OpenAPI 规范写入 <sample>.openapi.json，与其他子命令保持一致
            let spec_path = format!("{}.openapi.json", sample);
            std::fs::write(
                &spec_path,
                serde_json::to_string_pretty(&result.openapi_spec)?,
            )?;
            println!("  OpenAPI spec: {}", spec_path);
        }

        Commands::GenerateCli {
            main_help,
            sub_help,
            project,
            program,
        } => {
            let config = AppConfig::from_env();
            let pool = PgPool::connect(&config.database_url).await?;
            let repo = PgMetadataRepo::new(pool);
            repo.run_migrations().await?;

            // 读取主帮助文本
            let main_help_text = std::fs::read_to_string(&main_help)
                .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", main_help, e))?;

            // 解析 "name:/path/to/file" 格式的子命令帮助条目；
            // splitn(2, ':') 确保路径中含冒号（Windows 盘符）时不被误拆分
            let mut subcommand_helps: Vec<(String, String)> = Vec::new();
            for entry in &sub_help {
                let parts: Vec<&str> = entry.splitn(2, ':').collect();
                if parts.len() == 2 {
                    let help_text = std::fs::read_to_string(parts[1]).map_err(|e| {
                        anyhow::anyhow!("Failed to read sub_help {}: {}", parts[1], e)
                    })?;
                    subcommand_helps.push((parts[0].to_string(), help_text));
                }
            }

            let sub_refs: Vec<(&str, &str)> = subcommand_helps
                .iter()
                .map(|(n, h)| (n.as_str(), h.as_str()))
                .collect();

            // 创建项目记录，source_type 为 Cli 以区别于 WSDL 来源
            let project_obj = repo
                .create_project(&project, "CLI tool wrapper", "cli", SourceType::Cli)
                .await?;

            // 执行 CLI 生成流水线
            let result = GenerationPipeline::run_cli(
                &repo,
                project_obj.id,
                &program,
                &main_help_text,
                &sub_refs,
                llm_client.as_deref(),
            )
            .await?;

            println!("CLI Generation complete!");
            println!("  Contract ID: {}", result.contract_id);
            println!("  Routes created: {}", result.routes_count);

            // 将 OpenAPI 规范写入 <main_help>.openapi.json，与 generate 子命令保持一致
            let spec_path = format!("{}.openapi.json", main_help);
            std::fs::write(
                &spec_path,
                serde_json::to_string_pretty(&result.openapi_spec)?,
            )?;
            println!("  OpenAPI spec: {}", spec_path);
        }
    }

    Ok(())
}
