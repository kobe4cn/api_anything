use crate::cli_help::mapper::CliMapper;
use crate::cli_help::parser::CliHelpParser;
use crate::llm::client::LlmClient;
use crate::openapi::OpenApiGenerator;
use crate::ssh_sample::mapper::SshMapper;
use crate::ssh_sample::parser::SshSampleParser;
use crate::wsdl::{llm_mapper::LlmEnhancedMapper, mapper::WsdlMapper, parser::WsdlParser};
use api_anything_common::models::*;
use api_anything_metadata::MetadataRepo;
use serde_json::Value;
use uuid::Uuid;

pub struct GenerationPipeline;

#[derive(Debug)]
pub struct GenerationResult {
    pub contract_id: Uuid,
    pub routes_count: usize,
    pub openapi_spec: Value,
}

impl GenerationPipeline {
    /// WSDL 生成管道，接受可选的 LLM 客户端。
    /// 有 LLM 时使用 LlmEnhancedMapper 优化 HTTP 方法和路径设计，
    /// 无 LLM 时回退到确定性映射，保证功能完整。
    pub async fn run_wsdl(
        repo: &impl MetadataRepo,
        project_id: Uuid,
        wsdl_content: &str,
        llm: Option<&dyn LlmClient>,
    ) -> Result<GenerationResult, anyhow::Error> {
        // Stage 1: 解析 WSDL 文档，提取服务结构、消息定义和操作列表
        tracing::info!("Stage 1: Parsing WSDL");
        let wsdl = WsdlParser::parse(wsdl_content)?;

        // Stage 2: 根据 LLM 可用性选择映射策略
        let contract = if let Some(llm_client) = llm {
            tracing::info!("Stage 2: LLM-enhanced mapping to UnifiedContract");
            LlmEnhancedMapper::map(&wsdl, llm_client).await?
        } else {
            tracing::info!("Stage 2: Deterministic mapping to UnifiedContract");
            WsdlMapper::map(&wsdl)?
        };

        // Stage 3: 持久化合约，保存原始 schema 以便后续审计或重新解析
        tracing::info!("Stage 3: Persisting to metadata");
        let db_contract = repo
            .create_contract(
                project_id,
                "1.0.0",
                wsdl_content,
                &serde_json::to_value(&contract)?,
            )
            .await?;

        // Stage 4: 为每个操作创建后端绑定和路由记录，
        // 后端绑定携带 SOAP 转发所需的 endpoint_url、soapAction 等运行时参数
        let mut routes_count = 0;
        for op in &contract.operations {
            let endpoint_config = serde_json::json!({
                "url": op.endpoint_url,
                "soap_action": op.soap_action,
                "operation_name": op.name,
                "namespace": wsdl.target_namespace,
            });

            let binding = repo
                .create_backend_binding(ProtocolType::Soap, &endpoint_config, 30000)
                .await?;

            // SOAP 操作固定为 POST，此处 match 保留扩展性，
            // 方便将来支持 REST-style WSDL（如 HTTP binding）
            let method = match op.http_method.as_str() {
                "GET" => HttpMethod::Get,
                "PUT" => HttpMethod::Put,
                "DELETE" => HttpMethod::Delete,
                "PATCH" => HttpMethod::Patch,
                _ => HttpMethod::Post,
            };

            let request_schema = op
                .input
                .as_ref()
                .map(|m| m.schema.clone())
                .unwrap_or(serde_json::json!({}));
            let response_schema = op
                .output
                .as_ref()
                .map(|m| m.schema.clone())
                .unwrap_or(serde_json::json!({}));

            repo.create_route(
                db_contract.id,
                method,
                &op.path,
                &request_schema,
                &response_schema,
                &endpoint_config,
                binding.id,
            )
            .await?;
            routes_count += 1;
        }

        // Stage 5: 生成 OpenAPI 规范，供客户端 SDK 生成和文档展示使用
        tracing::info!("Stage 5: Generating OpenAPI spec");
        let openapi = OpenApiGenerator::generate(&contract);

        Ok(GenerationResult {
            contract_id: db_contract.id,
            routes_count,
            openapi_spec: openapi,
        })
    }

    /// CLI 管道：解析主帮助文本和各子命令帮助，映射为统一合约后走相同的持久化和 OpenAPI 生成流程。
    /// subcommand_helps 允许调用方按需传入子命令的详细帮助，未传入的子命令 options 列表保持为空。
    /// llm 参数为可选 LLM 客户端，当前保留为将来 CLI LLM 增强做入口。
    pub async fn run_cli(
        repo: &impl MetadataRepo,
        project_id: Uuid,
        program_name: &str,
        main_help: &str,
        subcommand_helps: &[(&str, &str)], // (name, help_text)
        _llm: Option<&dyn LlmClient>,
    ) -> Result<GenerationResult, anyhow::Error> {
        // Stage 1: 解析主帮助文本，获取程序名、子命令列表和全局选项
        tracing::info!("Stage 1: Parsing CLI help");
        let mut cli_def = CliHelpParser::parse_main(main_help)?;

        // 用各子命令的详细帮助丰富 options 列表；
        // 若子命令不在主帮助中则忽略，保证健壮性
        for (name, help) in subcommand_helps {
            let sub = CliHelpParser::parse_subcommand(help)?;
            if let Some(existing) = cli_def.subcommands.iter_mut().find(|s| s.name == *name) {
                existing.options = sub.options;
            }
        }

        // Stage 2: 将 CLI 定义映射为统一合约，HTTP 方法由子命令名语义推断
        tracing::info!("Stage 2: Mapping CLI to UnifiedContract");
        let contract = CliMapper::map(&cli_def, program_name)?;

        // Stage 3: 持久化合约，原始 schema 存主帮助文本，便于后续审计
        tracing::info!("Stage 3: Persisting to metadata");
        let db_contract = repo
            .create_contract(
                project_id,
                "1.0.0",
                main_help,
                &serde_json::to_value(&contract)?,
            )
            .await?;

        // Stage 4: 为每个子命令创建 CLI 协议绑定和路由记录。
        // endpoint_config 携带运行时执行命令所需的 program、subcommand 和 output_format，
        // 供 CLI 适配器（Phase2a-T4）在调度时拼接完整命令行
        let mut routes_count = 0;
        for op in &contract.operations {
            let endpoint_config = serde_json::json!({
                "program": program_name,
                "subcommand": op.name,
                "output_format": "json",
            });

            let binding = repo
                .create_backend_binding(ProtocolType::Cli, &endpoint_config, 30000)
                .await?;

            let method = match op.http_method.as_str() {
                "GET" => HttpMethod::Get,
                "PUT" => HttpMethod::Put,
                "DELETE" => HttpMethod::Delete,
                "PATCH" => HttpMethod::Patch,
                _ => HttpMethod::Post,
            };

            let request_schema = op
                .input
                .as_ref()
                .map(|m| m.schema.clone())
                .unwrap_or(serde_json::json!({}));
            let response_schema = op
                .output
                .as_ref()
                .map(|m| m.schema.clone())
                .unwrap_or(serde_json::json!({}));

            repo.create_route(
                db_contract.id,
                method,
                &op.path,
                &request_schema,
                &response_schema,
                &endpoint_config,
                binding.id,
            )
            .await?;
            routes_count += 1;
        }

        // Stage 5: 生成 OpenAPI 规范
        tracing::info!("Stage 5: Generating OpenAPI spec");
        let openapi = OpenApiGenerator::generate(&contract);

        Ok(GenerationResult {
            contract_id: db_contract.id,
            routes_count,
            openapi_spec: openapi,
        })
    }

    /// SSH 样本管道：解析 SSH 交互样本文本，映射为统一合约，持久化后生成 OpenAPI 规范。
    /// 流程结构与 run_wsdl / run_cli 保持一致，仅 Stage 1-2 使用 SSH 专属解析器和映射器。
    /// llm 参数为可选 LLM 客户端，当前保留为将来 SSH LLM 增强做入口。
    pub async fn run_ssh(
        repo: &impl MetadataRepo,
        project_id: Uuid,
        sample_text: &str,
        _llm: Option<&dyn LlmClient>,
    ) -> Result<GenerationResult, anyhow::Error> {
        // Stage 1: 解析 SSH 交互样本，提取 host、user 和命令列表
        tracing::info!("Stage 1: Parsing SSH sample");
        let ssh_def = SshSampleParser::parse(sample_text)?;

        // Stage 2: 将 SSH 定义映射为统一合约，HTTP 方法由命令首词语义决定
        tracing::info!("Stage 2: Mapping SSH sample to UnifiedContract");
        let contract = SshMapper::map(&ssh_def)?;

        // Stage 3: 持久化合约，原始样本文本保留以便后续审计
        tracing::info!("Stage 3: Persisting to metadata");
        let db_contract = repo
            .create_contract(
                project_id,
                "1.0.0",
                sample_text,
                &serde_json::to_value(&contract)?,
            )
            .await?;

        // Stage 4: 为每个 SSH 命令创建后端绑定和路由记录。
        // endpoint_config 携带 SSH 连接参数（host、user、command_template、output_format），
        // 供 SSH 适配器（Phase2b-T3）在调度时建立连接并执行命令
        let mut routes_count = 0;
        for (op, cmd) in contract.operations.iter().zip(ssh_def.commands.iter()) {
            let endpoint_config = serde_json::json!({
                "host": ssh_def.host,
                "user": ssh_def.user,
                "command_template": cmd.command_template,
                "output_format": cmd.output_format,
            });

            let binding = repo
                .create_backend_binding(ProtocolType::Ssh, &endpoint_config, 30000)
                .await?;

            let method = match op.http_method.as_str() {
                "GET" => HttpMethod::Get,
                "PUT" => HttpMethod::Put,
                "DELETE" => HttpMethod::Delete,
                "PATCH" => HttpMethod::Patch,
                _ => HttpMethod::Post,
            };

            let request_schema = op
                .input
                .as_ref()
                .map(|m| m.schema.clone())
                .unwrap_or(serde_json::json!({}));
            let response_schema = op
                .output
                .as_ref()
                .map(|m| m.schema.clone())
                .unwrap_or(serde_json::json!({}));

            repo.create_route(
                db_contract.id,
                method,
                &op.path,
                &request_schema,
                &response_schema,
                &endpoint_config,
                binding.id,
            )
            .await?;
            routes_count += 1;
        }

        // Stage 5: 生成 OpenAPI 规范
        tracing::info!("Stage 5: Generating OpenAPI spec");
        let openapi = OpenApiGenerator::generate(&contract);

        Ok(GenerationResult {
            contract_id: db_contract.id,
            routes_count,
            openapi_spec: openapi,
        })
    }
}
