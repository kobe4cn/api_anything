use crate::cli_help::mapper::CliMapper;
use crate::cli_help::parser::CliHelpParser;
use crate::codegen::CodegenEngine;
use crate::llm::client::LlmClient;
use crate::openapi::OpenApiGenerator;
use crate::ssh_sample::mapper::SshMapper;
use crate::ssh_sample::parser::SshSampleParser;
use crate::wsdl::{llm_mapper::LlmEnhancedMapper, mapper::WsdlMapper, parser::WsdlParser};
use api_anything_common::models::*;
use api_anything_metadata::MetadataRepo;
use serde_json::Value;
use std::path::PathBuf;
use uuid::Uuid;

pub struct GenerationPipeline;

#[derive(Debug)]
pub struct GenerationResult {
    pub contract_id: Uuid,
    pub routes_count: usize,
    pub openapi_spec: Value,
    /// LLM 代码生成模式下的编译产物路径
    pub plugin_path: Option<PathBuf>,
    /// LLM 代码生成模式下的 Rust 源码
    pub source_code: Option<String>,
}

impl GenerationPipeline {
    // =========================================================================
    // 新的统一入口 — LLM 驱动的代码生成流水线
    // =========================================================================

    /// 统一的生成入口，所有接口类型共用同一流程。
    ///
    /// 有 LLM 时走 7 阶段代码生成流水线（LLM 分析 -> 生成 Rust 代码 -> 编译 .so -> 测试 -> 文档 -> 观测 -> 产物存储）。
    /// 无 LLM 时打印明确警告，降级到确定性规则映射（仅 WSDL/CLI/SSH 支持）。
    pub async fn generate(
        repo: &impl MetadataRepo,
        project_id: Uuid,
        interface_type: &str,
        input_content: &str,
        project_name: &str,
        llm: Option<&dyn LlmClient>,
        workspace_dir: &std::path::Path,
        sdk_path: &std::path::Path,
    ) -> Result<GenerationResult, anyhow::Error> {
        let protocol_type = match interface_type {
            "soap" | "wsdl" => ProtocolType::Soap,
            "odata" | "openapi" | "rest" => ProtocolType::Http,
            "cli" => ProtocolType::Cli,
            "ssh" => ProtocolType::Ssh,
            "pty" => ProtocolType::Pty,
            _ => {
                return Err(anyhow::anyhow!(
                    "Unsupported interface type: {}",
                    interface_type
                ))
            }
        };

        if let Some(llm_client) = llm {
            Self::generate_with_llm(
                repo,
                project_id,
                interface_type,
                input_content,
                project_name,
                llm_client,
                workspace_dir,
                sdk_path,
                protocol_type,
            )
            .await
        } else {
            // 无 LLM 时降级，但发出明确警告让用户知道功能受限
            tracing::warn!(
                "No LLM configured — using basic deterministic mapping. \
                 This produces generic adapters without type-safe code generation. \
                 Configure LLM_PROVIDER and API key in .env for full functionality."
            );
            Self::generate_deterministic(
                repo,
                project_id,
                interface_type,
                input_content,
                project_name,
                protocol_type,
            )
            .await
        }
    }

    /// LLM 驱动的 7 阶段代码生成流水线
    async fn generate_with_llm(
        repo: &impl MetadataRepo,
        project_id: Uuid,
        interface_type: &str,
        input_content: &str,
        project_name: &str,
        llm: &dyn LlmClient,
        workspace_dir: &std::path::Path,
        sdk_path: &std::path::Path,
        protocol_type: ProtocolType,
    ) -> Result<GenerationResult, anyhow::Error> {
        // 确保工作目录存在
        std::fs::create_dir_all(workspace_dir)?;

        let engine = CodegenEngine::new(
            llm,
            workspace_dir.to_path_buf(),
            sdk_path.to_path_buf(),
        );

        // Stage 1-7: 完整的代码生成流水线
        let codegen_result = engine
            .generate(interface_type, input_content, project_name)
            .await?;

        // 持久化到数据库
        tracing::info!("Persisting generated artifacts to metadata");
        let db_contract = repo
            .create_contract(
                project_id,
                "1.0.0",
                input_content,
                &serde_json::json!({
                    "generator": "llm_codegen",
                    "llm_model": llm.model_name(),
                    "interface_type": interface_type,
                    "routes_count": codegen_result.routes.len(),
                }),
            )
            .await?;

        let mut routes_count = 0;
        for route in &codegen_result.routes {
            let endpoint_config = serde_json::json!({
                "plugin_path": codegen_result.plugin_path.to_str(),
                "interface_type": interface_type,
                "operation_name": route.operation_name,
            });

            let method = match route.method.to_uppercase().as_str() {
                "GET" => HttpMethod::Get,
                "PUT" => HttpMethod::Put,
                "DELETE" => HttpMethod::Delete,
                "PATCH" => HttpMethod::Patch,
                _ => HttpMethod::Post,
            };

            let binding = repo
                .create_backend_binding(protocol_type.clone(), &endpoint_config, 30000)
                .await?;

            repo.create_route(
                db_contract.id,
                method,
                &route.path,
                &route.request_schema,
                &route.response_schema,
                &endpoint_config,
                binding.id,
            )
            .await?;
            routes_count += 1;
        }

        Ok(GenerationResult {
            contract_id: db_contract.id,
            routes_count,
            openapi_spec: codegen_result.openapi_spec,
            plugin_path: Some(codegen_result.plugin_path),
            source_code: Some(codegen_result.source_code),
        })
    }

    /// 无 LLM 时的确定性降级：沿用原有的解析器 + 映射器逻辑。
    /// 仅支持 WSDL/CLI/SSH 三种已实现解析器的接口类型。
    async fn generate_deterministic(
        repo: &impl MetadataRepo,
        project_id: Uuid,
        interface_type: &str,
        input_content: &str,
        project_name: &str,
        protocol_type: ProtocolType,
    ) -> Result<GenerationResult, anyhow::Error> {
        match interface_type {
            "soap" | "wsdl" => {
                // 复用现有 WSDL 确定性管道
                let wsdl = WsdlParser::parse(input_content)?;
                let contract = WsdlMapper::map(&wsdl)?;
                Self::persist_unified_contract(
                    repo,
                    project_id,
                    input_content,
                    &contract,
                    protocol_type,
                    project_name,
                )
                .await
            }
            "cli" => {
                // CLI 降级只解析主帮助，不支持子命令详情
                let cli_def = CliHelpParser::parse_main(input_content)?;
                let contract = CliMapper::map(&cli_def, project_name)?;
                Self::persist_unified_contract(
                    repo,
                    project_id,
                    input_content,
                    &contract,
                    protocol_type,
                    project_name,
                )
                .await
            }
            "ssh" => {
                let ssh_def = SshSampleParser::parse(input_content)?;
                let contract = SshMapper::map(&ssh_def)?;
                Self::persist_unified_contract(
                    repo,
                    project_id,
                    input_content,
                    &contract,
                    protocol_type,
                    project_name,
                )
                .await
            }
            _ => Err(anyhow::anyhow!(
                "Interface type '{}' requires LLM — no deterministic fallback available. \
                 Configure LLM_PROVIDER and API key in .env.",
                interface_type
            )),
        }
    }

    /// 将统一合约持久化到数据库并生成 OpenAPI 规范（确定性模式使用）
    async fn persist_unified_contract(
        repo: &impl MetadataRepo,
        project_id: Uuid,
        original_schema: &str,
        contract: &crate::unified_contract::UnifiedContract,
        protocol_type: ProtocolType,
        _project_name: &str,
    ) -> Result<GenerationResult, anyhow::Error> {
        let db_contract = repo
            .create_contract(
                project_id,
                "1.0.0",
                original_schema,
                &serde_json::to_value(contract)?,
            )
            .await?;

        let mut routes_count = 0;
        for op in &contract.operations {
            let endpoint_config = serde_json::json!({
                "url": op.endpoint_url,
                "soap_action": op.soap_action,
                "operation_name": op.name,
            });

            let binding = repo
                .create_backend_binding(protocol_type.clone(), &endpoint_config, 30000)
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

        let openapi = OpenApiGenerator::generate(contract);

        Ok(GenerationResult {
            contract_id: db_contract.id,
            routes_count,
            openapi_spec: openapi,
            plugin_path: None,
            source_code: None,
        })
    }

    // =========================================================================
    // 保留原有入口（向后兼容），内部委托给新的统一 generate() 或确定性逻辑
    // =========================================================================

    /// WSDL 生成管道（保留原签名，向后兼容现有调用方和测试）。
    /// 有 LLM 时使用 LlmEnhancedMapper 优化 HTTP 方法和路径设计，
    /// 无 LLM 时回退到确定性映射，保证功能完整。
    pub async fn run_wsdl(
        repo: &impl MetadataRepo,
        project_id: Uuid,
        wsdl_content: &str,
        llm: Option<&dyn LlmClient>,
    ) -> Result<GenerationResult, anyhow::Error> {
        // 保留原有确定性逻辑，因为 LLM 增强在此路径中仅优化路由映射而非生成完整代码
        tracing::info!("Stage 1: Parsing WSDL");
        let wsdl = WsdlParser::parse(wsdl_content)?;

        let contract = if let Some(llm_client) = llm {
            tracing::info!("Stage 2: LLM-enhanced mapping to UnifiedContract");
            LlmEnhancedMapper::map(&wsdl, llm_client).await?
        } else {
            tracing::info!("Stage 2: Deterministic mapping to UnifiedContract");
            WsdlMapper::map(&wsdl)?
        };

        tracing::info!("Stage 3: Persisting to metadata");
        let db_contract = repo
            .create_contract(
                project_id,
                "1.0.0",
                wsdl_content,
                &serde_json::to_value(&contract)?,
            )
            .await?;

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

        tracing::info!("Stage 5: Generating OpenAPI spec");
        let openapi = OpenApiGenerator::generate(&contract);

        Ok(GenerationResult {
            contract_id: db_contract.id,
            routes_count,
            openapi_spec: openapi,
            plugin_path: None,
            source_code: None,
        })
    }

    /// CLI 管道（保留原签名，向后兼容）
    pub async fn run_cli(
        repo: &impl MetadataRepo,
        project_id: Uuid,
        program_name: &str,
        main_help: &str,
        subcommand_helps: &[(&str, &str)],
        _llm: Option<&dyn LlmClient>,
    ) -> Result<GenerationResult, anyhow::Error> {
        tracing::info!("Stage 1: Parsing CLI help");
        let mut cli_def = CliHelpParser::parse_main(main_help)?;

        for (name, help) in subcommand_helps {
            let sub = CliHelpParser::parse_subcommand(help)?;
            if let Some(existing) = cli_def.subcommands.iter_mut().find(|s| s.name == *name) {
                existing.options = sub.options;
            }
        }

        tracing::info!("Stage 2: Mapping CLI to UnifiedContract");
        let contract = CliMapper::map(&cli_def, program_name)?;

        tracing::info!("Stage 3: Persisting to metadata");
        let db_contract = repo
            .create_contract(
                project_id,
                "1.0.0",
                main_help,
                &serde_json::to_value(&contract)?,
            )
            .await?;

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

        tracing::info!("Stage 5: Generating OpenAPI spec");
        let openapi = OpenApiGenerator::generate(&contract);

        Ok(GenerationResult {
            contract_id: db_contract.id,
            routes_count,
            openapi_spec: openapi,
            plugin_path: None,
            source_code: None,
        })
    }

    /// SSH 样本管道（保留原签名，向后兼容）
    pub async fn run_ssh(
        repo: &impl MetadataRepo,
        project_id: Uuid,
        sample_text: &str,
        _llm: Option<&dyn LlmClient>,
    ) -> Result<GenerationResult, anyhow::Error> {
        tracing::info!("Stage 1: Parsing SSH sample");
        let ssh_def = SshSampleParser::parse(sample_text)?;

        tracing::info!("Stage 2: Mapping SSH sample to UnifiedContract");
        let contract = SshMapper::map(&ssh_def)?;

        tracing::info!("Stage 3: Persisting to metadata");
        let db_contract = repo
            .create_contract(
                project_id,
                "1.0.0",
                sample_text,
                &serde_json::to_value(&contract)?,
            )
            .await?;

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

        tracing::info!("Stage 5: Generating OpenAPI spec");
        let openapi = OpenApiGenerator::generate(&contract);

        Ok(GenerationResult {
            contract_id: db_contract.id,
            routes_count,
            openapi_spec: openapi,
            plugin_path: None,
            source_code: None,
        })
    }
}
