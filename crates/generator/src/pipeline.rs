use crate::wsdl::{parser::WsdlParser, mapper::WsdlMapper, llm_mapper::LlmEnhancedMapper};
use crate::llm::client::LlmClient;
use crate::openapi::OpenApiGenerator;
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
    pub async fn run_wsdl(
        repo: &impl MetadataRepo,
        project_id: Uuid,
        wsdl_content: &str,
    ) -> Result<GenerationResult, anyhow::Error> {
        // Stage 1: 解析 WSDL 文档，提取服务结构、消息定义和操作列表
        tracing::info!("Stage 1: Parsing WSDL");
        let wsdl = WsdlParser::parse(wsdl_content)?;

        // Stage 2: 将 WSDL 结构映射为统一合约中间表示，屏蔽 SOAP 特有细节
        tracing::info!("Stage 2: Mapping to UnifiedContract");
        let contract = WsdlMapper::map(&wsdl)?;

        // Stage 3: 持久化合约，保存原始 schema 以便后续审计或重新解析
        tracing::info!("Stage 3: Persisting to metadata");
        let db_contract = repo.create_contract(
            project_id,
            "1.0.0",
            wsdl_content,
            &serde_json::to_value(&contract)?,
        ).await?;

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

            let binding = repo.create_backend_binding(
                ProtocolType::Soap,
                &endpoint_config,
                30000,
            ).await?;

            // SOAP 操作固定为 POST，此处 match 保留扩展性，
            // 方便将来支持 REST-style WSDL（如 HTTP binding）
            let method = match op.http_method.as_str() {
                "GET" => HttpMethod::Get,
                "PUT" => HttpMethod::Put,
                "DELETE" => HttpMethod::Delete,
                "PATCH" => HttpMethod::Patch,
                _ => HttpMethod::Post,
            };

            let request_schema = op.input.as_ref()
                .map(|m| m.schema.clone())
                .unwrap_or(serde_json::json!({}));
            let response_schema = op.output.as_ref()
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
            ).await?;
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

    /// 与 run_wsdl 流程相同，但 Stage 2 使用 LLM 增强映射器优化 HTTP 方法和路径。
    /// 若 LLM 不可用，LlmEnhancedMapper 内部会自动降级到确定性映射，保证流程不中断。
    pub async fn run_wsdl_with_llm(
        repo: &impl MetadataRepo,
        project_id: Uuid,
        wsdl_content: &str,
        llm: &dyn LlmClient,
    ) -> Result<GenerationResult, anyhow::Error> {
        // Stage 1: 解析 WSDL 文档
        tracing::info!("Stage 1: Parsing WSDL");
        let wsdl = WsdlParser::parse(wsdl_content)?;

        // Stage 2: LLM 增强映射，相比 run_wsdl 的确定性映射，
        // 此处尝试让 LLM 优化 HTTP 方法语义和路径命名风格
        tracing::info!("Stage 2: LLM-enhanced mapping to UnifiedContract");
        let contract = LlmEnhancedMapper::map(&wsdl, llm).await?;

        // Stage 3: 持久化合约
        tracing::info!("Stage 3: Persisting to metadata");
        let db_contract = repo.create_contract(
            project_id,
            "1.0.0",
            wsdl_content,
            &serde_json::to_value(&contract)?,
        ).await?;

        // Stage 4: 创建后端绑定和路由记录
        let mut routes_count = 0;
        for op in &contract.operations {
            let endpoint_config = serde_json::json!({
                "url": op.endpoint_url,
                "soap_action": op.soap_action,
                "operation_name": op.name,
                "namespace": wsdl.target_namespace,
            });

            let binding = repo.create_backend_binding(
                ProtocolType::Soap,
                &endpoint_config,
                30000,
            ).await?;

            let method = match op.http_method.as_str() {
                "GET" => HttpMethod::Get,
                "PUT" => HttpMethod::Put,
                "DELETE" => HttpMethod::Delete,
                "PATCH" => HttpMethod::Patch,
                _ => HttpMethod::Post,
            };

            let request_schema = op.input.as_ref()
                .map(|m| m.schema.clone())
                .unwrap_or(serde_json::json!({}));
            let response_schema = op.output.as_ref()
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
            ).await?;
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
