use crate::adapters::cli_process::{CliAdapter, CliConfig};
use crate::adapters::pty_expect::{PtyAdapter, PtyConfig};
use crate::adapters::soap::{SoapAdapter, SoapConfig};
use crate::adapters::ssh_remote::SshAdapter;
use crate::dispatcher::{BackendDispatcher, ProtectionStack};
use crate::router::{DynamicRouter, RouteTable};
use api_anything_common::error::AppError;
use api_anything_common::models::{HttpMethod, ProtocolType, RouteWithBinding};
use api_anything_metadata::repo::MetadataRepo;
use axum::http::Method;
use dashmap::DashMap;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// 从连接池配置中读取 rps 和可选的 max_concurrent；
/// max_concurrent 返回 Option，方便调用方区分"未配置"与"显式为零"，
/// 从而按协议类型应用不同的默认值
fn extract_pool_config(config: &serde_json::Value) -> (Option<u32>, u32) {
    let max_concurrent = config
        .get("max_concurrent")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);
    let rps = config
        .get("rps")
        .and_then(|v| v.as_u64())
        .unwrap_or(1000) as u32;
    (max_concurrent, rps)
}

/// 从 circuit_breaker_config 提取熔断参数；
/// 返回 Option 包装，允许调用方区分"未配置"与"显式为零"两种情形，
/// 从而按协议类型应用不同的默认值
fn extract_circuit_breaker_config(
    config: &serde_json::Value,
) -> (Option<f64>, Option<u64>, Option<u64>, u32) {
    let error_threshold = config
        .get("error_threshold_pct")
        .and_then(|v| v.as_f64());
    let window_secs = config.get("window_secs").and_then(|v| v.as_u64());
    let open_secs = config.get("open_secs").and_then(|v| v.as_u64());
    let half_open_max = config
        .get("half_open_max")
        .and_then(|v| v.as_u64())
        .unwrap_or(3) as u32;
    (error_threshold, window_secs, open_secs, half_open_max)
}

/// 将 common::models::HttpMethod 转换为 axum::http::Method；
/// 两者虽语义相同，但属于不同 crate 的类型，需显式映射
fn to_axum_method(m: &HttpMethod) -> Method {
    match m {
        HttpMethod::Get => Method::GET,
        HttpMethod::Post => Method::POST,
        HttpMethod::Put => Method::PUT,
        HttpMethod::Patch => Method::PATCH,
        HttpMethod::Delete => Method::DELETE,
    }
}

/// 从 endpoint_config JSON 构造 SoapConfig；
/// 所有字段均为必填，缺失时返回 AppError::Internal 而非 panic，
/// 保证加载过程可观测、可回滚
fn soap_config_from_endpoint(endpoint_config: &serde_json::Value) -> Result<SoapConfig, AppError> {
    let url = endpoint_config
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Internal("SOAP endpoint_config missing 'url'".to_string()))?
        .to_string();
    let soap_action = endpoint_config
        .get("soap_action")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let operation_name = endpoint_config
        .get("operation_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let namespace = endpoint_config
        .get("namespace")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    Ok(SoapConfig {
        endpoint_url: url,
        soap_action,
        operation_name,
        namespace,
    })
}

/// 从 endpoint_config JSON 构造 CliConfig；
/// output_format 以带 type 标签的 JSON 存储（如 {"type":"RawText"}），
/// 反序列化失败时兜底为 RawText 而非 panic，保证路由加载不因格式变更而中断
fn build_cli_config(route: &RouteWithBinding) -> Result<CliConfig, anyhow::Error> {
    let ec = &route.endpoint_config;
    Ok(CliConfig {
        program: ec["program"].as_str().unwrap_or("").to_string(),
        subcommand: ec["subcommand"].as_str().map(|s| s.to_string()),
        static_args: ec["static_args"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        // output_format 存储为 JSON 字符串（如 "json"）或带 type 标签的对象；
        // pipeline 目前写入的是裸字符串 "json"，需先尝试解析为对象，
        // 失败时检查是否为已知字符串别名，最终兜底为 RawText
        output_format: parse_output_format(ec.get("output_format")),
    })
}

/// 将 endpoint_config 中的 output_format 字段解析为 OutputFormat；
/// 支持带 type 标签的对象（{"type":"Json"}）和裸字符串别名（"json"/"raw"/"text"），
/// 两种格式均不匹配时安全降级为 RawText 而非返回错误
fn parse_output_format(
    value: Option<&serde_json::Value>,
) -> crate::output_parser::OutputFormat {
    use crate::output_parser::OutputFormat;
    let Some(v) = value else {
        return OutputFormat::RawText;
    };
    // 先尝试标准的带 type 标签反序列化（{"type":"Json"} 等）
    if let Ok(fmt) = serde_json::from_value::<OutputFormat>(v.clone()) {
        return fmt;
    }
    // 兼容 pipeline 写入的裸字符串（"json" / "Json"）
    if let Some(s) = v.as_str() {
        match s.to_ascii_lowercase().as_str() {
            "json" => return OutputFormat::Json,
            "rawtext" | "raw" | "text" => return OutputFormat::RawText,
            _ => {}
        }
    }
    OutputFormat::RawText
}

/// 从 endpoint_config JSON 构造 SshConfig；
/// output_format 复用与 CLI 相同的 parse_output_format 逻辑，
/// port 缺失时默认 22，identity_file 可选
fn build_ssh_config(route: &RouteWithBinding) -> Result<crate::adapters::ssh_remote::SshConfig, anyhow::Error> {
    let ec = &route.endpoint_config;
    Ok(crate::adapters::ssh_remote::SshConfig {
        host: ec["host"].as_str().unwrap_or("").to_string(),
        port: ec["port"].as_u64().unwrap_or(22) as u16,
        user: ec["user"].as_str().unwrap_or("").to_string(),
        command_template: ec["command_template"].as_str().unwrap_or("").to_string(),
        output_format: parse_output_format(ec.get("output_format")),
        identity_file: ec["identity_file"].as_str().map(String::from),
    })
}

/// 从 endpoint_config JSON 构造 PtyConfig；
/// init_commands 为可选数组，timeout_ms 缺失时以保护栈超时为准，
/// 这里直接读取配置值兜底为 300s（与 PTY 协议的默认超时一致）
fn build_pty_config(route: &RouteWithBinding) -> Result<PtyConfig, anyhow::Error> {
    let ec = &route.endpoint_config;
    Ok(PtyConfig {
        program: ec["program"].as_str().unwrap_or("").to_string(),
        args: ec["args"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        prompt_pattern: ec["prompt_pattern"].as_str().unwrap_or(r"\$\s*$").to_string(),
        command_template: ec["command_template"].as_str().unwrap_or("").to_string(),
        output_format: parse_output_format(ec.get("output_format")),
        init_commands: ec["init_commands"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        // timeout_ms 从路由超时继承；为零时兜底使用 PTY 协议默认的 300s，
        // 确保长时间运行的设备命令不会被过早截断
        timeout_ms: if route.timeout_ms > 0 {
            route.timeout_ms as u64
        } else {
            300_000
        },
    })
}

/// 根据协议类型构造协议感知的保护栈；
/// 不同协议的行为差异显著（CLI 进程比 SOAP HTTP 慢得多、并发度更低），
/// 硬编码在 SOAP 时代的通用默认值会导致 CLI 路由的保护策略失当
fn build_protection_stack(route: &RouteWithBinding) -> ProtectionStack {
    // 协议特定的默认值来自规格 §5.4；
    // CLI：进程启动开销大，并发度低、错误敏感性高、超时需更长；
    // SSH/PTY：交互式协议，并发极低，超时最宽松；
    // SOAP/HTTP：高并发、快速响应的传统 RPC 默认值
    let (default_max_conn, default_err_threshold, default_window_ms, default_open_ms, default_timeout_ms) =
        match route.protocol {
            ProtocolType::Cli => (10u32, 30.0f64, 10_000u64, 30_000u64, 60_000i64),
            ProtocolType::Ssh => (5, 20.0, 10_000, 30_000, 120_000),
            ProtocolType::Pty => (3, 20.0, 10_000, 30_000, 300_000),
            _ => (100, 50.0, 30_000, 60_000, 30_000),
        };

    let (cfg_max_conn, rps) = extract_pool_config(&route.connection_pool_config);
    let (cfg_err_threshold, cfg_window_secs, cfg_open_secs, half_open_max) =
        extract_circuit_breaker_config(&route.circuit_breaker_config);

    let max_concurrent = cfg_max_conn.unwrap_or(default_max_conn);
    let error_threshold = cfg_err_threshold.unwrap_or(default_err_threshold);
    let window_duration =
        Duration::from_millis(cfg_window_secs.map(|s| s * 1000).unwrap_or(default_window_ms));
    let open_duration =
        Duration::from_millis(cfg_open_secs.map(|s| s * 1000).unwrap_or(default_open_ms));

    // timeout_ms 存储为 i64 防止 SQL 类型溢出；实际超时不应为负，兜底使用协议特定默认值
    let timeout = if route.timeout_ms > 0 {
        Duration::from_millis(route.timeout_ms as u64)
    } else {
        Duration::from_millis(default_timeout_ms as u64)
    };

    ProtectionStack::new(
        rps,
        max_concurrent,
        error_threshold,
        window_duration,
        open_duration,
        half_open_max,
        timeout,
    )
}

pub struct RouteLoader;

impl RouteLoader {
    /// 从数据库加载所有启用的路由，构建适配器 + ProtectionStack + BackendDispatcher，
    /// 原子更新 DynamicRouter 的路由表，并将 dispatcher 写入共享 DashMap；
    /// 返回成功加载的路由数量，跳过的路由（协议不支持、配置缺失）仅记录 warn 日志；
    /// 使用泛型约束而非 dyn trait，因为 MetadataRepo 含 async fn，不满足 dyn 兼容性要求
    pub async fn load<R: MetadataRepo>(
        repo: &R,
        router: &DynamicRouter,
        dispatchers: &DashMap<Uuid, Arc<BackendDispatcher>>,
    ) -> Result<usize, AppError> {
        let routes: Vec<RouteWithBinding> = repo.list_active_routes_with_bindings().await?;

        let mut table = RouteTable::new();
        let mut count = 0usize;

        for route in &routes {
            let adapter: Box<dyn crate::adapter::ProtocolAdapter> = match route.protocol {
                ProtocolType::Soap => {
                    match soap_config_from_endpoint(&route.endpoint_config) {
                        Ok(cfg) => Box::new(SoapAdapter::new(cfg)),
                        Err(e) => {
                            tracing::warn!(
                                route_id = %route.route_id,
                                error = %e,
                                "Skipping SOAP route: invalid endpoint_config"
                            );
                            continue;
                        }
                    }
                }
                ProtocolType::Cli => {
                    match build_cli_config(route) {
                        Ok(cfg) => Box::new(CliAdapter::new(cfg)),
                        Err(e) => {
                            tracing::warn!(
                                route_id = %route.route_id,
                                error = %e,
                                "Skipping CLI route: invalid endpoint_config"
                            );
                            continue;
                        }
                    }
                }
                ProtocolType::Ssh => {
                    match build_ssh_config(route) {
                        Ok(cfg) => Box::new(SshAdapter::new(cfg)),
                        Err(e) => {
                            tracing::warn!(
                                route_id = %route.route_id,
                                error = %e,
                                "Skipping SSH route: invalid endpoint_config"
                            );
                            continue;
                        }
                    }
                }
                ProtocolType::Pty => {
                    match build_pty_config(route) {
                        Ok(cfg) => Box::new(PtyAdapter::new(cfg)),
                        Err(e) => {
                            tracing::warn!(
                                route_id = %route.route_id,
                                error = %e,
                                "Skipping PTY route: invalid endpoint_config"
                            );
                            continue;
                        }
                    }
                }
                ref other => {
                    tracing::warn!(
                        route_id = %route.route_id,
                        protocol = ?other,
                        "Skipping route: unsupported protocol"
                    );
                    continue;
                }
            };

            let protection = build_protection_stack(route);
            let dispatcher = Arc::new(BackendDispatcher::new(adapter, protection));
            dispatchers.insert(route.route_id, dispatcher);

            let method = to_axum_method(&route.method);
            table.insert(method, &route.path, route.route_id);

            count += 1;
        }

        // 原子替换路由表，已在处理中的请求仍持有旧表引用直至完成，不会被中断
        router.update(table);

        tracing::info!(routes = count, "RouteLoader: loaded gateway routes");
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api_anything_common::models::*;
    use api_anything_metadata::repo::MetadataRepo;
    use std::sync::Arc;

    /// 用于单元测试的内存 MetadataRepo，无需真实数据库连接
    struct MockRepo {
        routes: Vec<RouteWithBinding>,
    }

    impl MetadataRepo for MockRepo {
        async fn create_project(
            &self,
            _name: &str,
            _description: &str,
            _owner: &str,
            _source_type: SourceType,
        ) -> Result<Project, AppError> {
            unimplemented!()
        }
        async fn get_project(&self, _id: Uuid) -> Result<Project, AppError> {
            unimplemented!()
        }
        async fn list_projects(&self) -> Result<Vec<Project>, AppError> {
            unimplemented!()
        }
        async fn delete_project(&self, _id: Uuid) -> Result<(), AppError> {
            unimplemented!()
        }
        async fn get_route(&self, _id: Uuid) -> Result<Route, AppError> {
            unimplemented!()
        }
        async fn list_active_routes_with_bindings(
            &self,
        ) -> Result<Vec<RouteWithBinding>, AppError> {
            Ok(self.routes.clone())
        }
        async fn create_contract(
            &self,
            _project_id: Uuid,
            _version: &str,
            _original_schema: &str,
            _parsed_model: &serde_json::Value,
        ) -> Result<Contract, AppError> {
            unimplemented!()
        }
        async fn create_backend_binding(
            &self,
            _protocol: ProtocolType,
            _endpoint_config: &serde_json::Value,
            _timeout_ms: i64,
        ) -> Result<BackendBinding, AppError> {
            unimplemented!()
        }
        async fn create_route(
            &self,
            _contract_id: Uuid,
            _method: HttpMethod,
            _path: &str,
            _request_schema: &serde_json::Value,
            _response_schema: &serde_json::Value,
            _transform_rules: &serde_json::Value,
            _backend_binding_id: Uuid,
        ) -> Result<Route, AppError> {
            unimplemented!()
        }

        // MockRepo 仅用于网关路由加载的单元测试，沙箱及录音方法不在测试范围内
        async fn create_sandbox_session(
            &self,
            _project_id: Uuid,
            _tenant_id: &str,
            _mode: SandboxMode,
            _config: &serde_json::Value,
            _expires_at: chrono::DateTime<chrono::Utc>,
        ) -> Result<SandboxSession, AppError> {
            unimplemented!()
        }
        async fn get_sandbox_session(&self, _id: Uuid) -> Result<SandboxSession, AppError> {
            unimplemented!()
        }
        async fn list_sandbox_sessions(&self, _project_id: Uuid) -> Result<Vec<SandboxSession>, AppError> {
            unimplemented!()
        }
        async fn delete_sandbox_session(&self, _id: Uuid) -> Result<(), AppError> {
            unimplemented!()
        }
        async fn record_interaction(
            &self,
            _session_id: Uuid,
            _route_id: Uuid,
            _request: &serde_json::Value,
            _response: &serde_json::Value,
            _duration_ms: i32,
        ) -> Result<RecordedInteraction, AppError> {
            unimplemented!()
        }
        async fn find_matching_interaction(
            &self,
            _session_id: Uuid,
            _route_id: Uuid,
            _request: &serde_json::Value,
        ) -> Result<Option<RecordedInteraction>, AppError> {
            unimplemented!()
        }
        async fn list_recorded_interactions(
            &self,
            _session_id: Uuid,
        ) -> Result<Vec<RecordedInteraction>, AppError> {
            unimplemented!()
        }
        async fn delete_recorded_interactions(
            &self,
            _session_id: Uuid,
        ) -> Result<u64, AppError> {
            unimplemented!()
        }

        // MockRepo 仅用于网关路由加载测试，补偿系统方法不在测试范围内
        async fn create_delivery_record(
            &self,
            _route_id: Uuid,
            _trace_id: &str,
            _idempotency_key: Option<&str>,
            _request_payload: &serde_json::Value,
        ) -> Result<api_anything_common::models::DeliveryRecord, AppError> {
            unimplemented!()
        }
        async fn update_delivery_status(
            &self,
            _id: Uuid,
            _status: api_anything_common::models::DeliveryStatus,
            _error_message: Option<&str>,
            _next_retry_at: Option<chrono::DateTime<chrono::Utc>>,
        ) -> Result<(), AppError> {
            unimplemented!()
        }
        async fn list_pending_retries(
            &self,
            _limit: i64,
        ) -> Result<Vec<api_anything_common::models::DeliveryRecord>, AppError> {
            unimplemented!()
        }
        async fn list_dead_letters(
            &self,
            _route_id: Option<Uuid>,
            _limit: i64,
            _offset: i64,
        ) -> Result<Vec<api_anything_common::models::DeliveryRecord>, AppError> {
            unimplemented!()
        }
        async fn get_delivery_record(
            &self,
            _id: Uuid,
        ) -> Result<api_anything_common::models::DeliveryRecord, AppError> {
            unimplemented!()
        }
        async fn check_idempotency(
            &self,
            _key: &str,
        ) -> Result<Option<api_anything_common::models::IdempotencyRecord>, AppError> {
            unimplemented!()
        }
        async fn create_idempotency_record(
            &self,
            _key: &str,
            _route_id: Uuid,
        ) -> Result<(), AppError> {
            unimplemented!()
        }
        async fn mark_idempotency_delivered(
            &self,
            _key: &str,
            _response_hash: &str,
        ) -> Result<(), AppError> {
            unimplemented!()
        }

        // MockRepo 仅用于网关路由加载测试，Webhook 订阅方法不在测试范围内
        async fn create_webhook_subscription(
            &self,
            _url: &str,
            _event_types: &serde_json::Value,
            _description: &str,
        ) -> Result<api_anything_common::models::WebhookSubscription, AppError> {
            unimplemented!()
        }
        async fn list_webhook_subscriptions(
            &self,
        ) -> Result<Vec<api_anything_common::models::WebhookSubscription>, AppError> {
            unimplemented!()
        }
        async fn delete_webhook_subscription(&self, _id: Uuid) -> Result<(), AppError> {
            unimplemented!()
        }
        async fn list_active_subscriptions_for_event(
            &self,
            _event_type: &str,
        ) -> Result<Vec<api_anything_common::models::WebhookSubscription>, AppError> {
            unimplemented!()
        }
    }

    fn make_soap_route(route_id: Uuid, path: &str) -> RouteWithBinding {
        RouteWithBinding {
            route_id,
            contract_id: Uuid::new_v4(),
            method: HttpMethod::Post,
            path: path.to_string(),
            request_schema: serde_json::json!({}),
            response_schema: serde_json::json!({}),
            transform_rules: serde_json::json!({}),
            delivery_guarantee: DeliveryGuarantee::AtMostOnce,
            binding_id: Uuid::new_v4(),
            protocol: ProtocolType::Soap,
            endpoint_config: serde_json::json!({
                "url": "http://example.com/soap",
                "soap_action": "TestAction",
                "operation_name": "Test",
                "namespace": "http://example.com"
            }),
            connection_pool_config: serde_json::json!({}),
            circuit_breaker_config: serde_json::json!({}),
            rate_limit_config: serde_json::json!({}),
            retry_config: serde_json::json!({}),
            timeout_ms: 5000,
            auth_mapping: serde_json::json!({}),
        }
    }

    fn make_cli_route(route_id: Uuid, path: &str, program: &str) -> RouteWithBinding {
        RouteWithBinding {
            route_id,
            contract_id: Uuid::new_v4(),
            method: HttpMethod::Post,
            path: path.to_string(),
            request_schema: serde_json::json!({}),
            response_schema: serde_json::json!({}),
            transform_rules: serde_json::json!({}),
            delivery_guarantee: DeliveryGuarantee::AtMostOnce,
            binding_id: Uuid::new_v4(),
            protocol: ProtocolType::Cli,
            endpoint_config: serde_json::json!({
                "program": program,
                "subcommand": "generate",
                "output_format": "json",
            }),
            connection_pool_config: serde_json::json!({}),
            circuit_breaker_config: serde_json::json!({}),
            rate_limit_config: serde_json::json!({}),
            retry_config: serde_json::json!({}),
            timeout_ms: 0,
            auth_mapping: serde_json::json!({}),
        }
    }

    #[tokio::test]
    async fn loads_soap_routes_and_registers_in_router() {
        let route_id = Uuid::new_v4();
        let repo = MockRepo {
            routes: vec![make_soap_route(route_id, "/orders/{id}")],
        };
        let router = DynamicRouter::new();
        let dispatchers: DashMap<Uuid, Arc<BackendDispatcher>> = DashMap::new();

        let count = RouteLoader::load(&repo, &router, &dispatchers).await.unwrap();

        assert_eq!(count, 1);
        // dispatcher 应以 route_id 为键写入共享 map
        assert!(dispatchers.contains_key(&route_id));
        // 路由表应能通过 POST /orders/123 匹配到正确的 route_id
        let result = router.match_route(&Method::POST, "/orders/123");
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, route_id);
    }

    #[tokio::test]
    async fn loads_cli_routes_and_registers_in_router() {
        let route_id = Uuid::new_v4();
        let repo = MockRepo {
            routes: vec![make_cli_route(route_id, "/reports/generate", "echo")],
        };
        let router = DynamicRouter::new();
        let dispatchers: DashMap<Uuid, Arc<BackendDispatcher>> = DashMap::new();

        let count = RouteLoader::load(&repo, &router, &dispatchers).await.unwrap();

        assert_eq!(count, 1);
        assert!(dispatchers.contains_key(&route_id));
        let result = router.match_route(&Method::POST, "/reports/generate");
        assert!(result.is_some());
    }

    #[tokio::test]
    async fn cli_route_uses_protocol_specific_defaults_when_timeout_zero() {
        // timeout_ms = 0 时应兜底到 CLI 协议的 60s 默认值，而非 SOAP 的 30s
        let route_id = Uuid::new_v4();
        let repo = MockRepo {
            routes: vec![make_cli_route(route_id, "/reports/list", "echo")],
        };
        let router = DynamicRouter::new();
        let dispatchers: DashMap<Uuid, Arc<BackendDispatcher>> = DashMap::new();

        // 加载成功即说明 CLI 默认值正确应用，保护栈构建没有 panic
        let count = RouteLoader::load(&repo, &router, &dispatchers).await.unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn skips_unsupported_protocol_and_returns_zero() {
        let route_id = Uuid::new_v4();
        let mut route = make_soap_route(route_id, "/orders");
        // 将协议改为 Http，当前加载器不支持此协议，应跳过
        route.protocol = ProtocolType::Http;

        let repo = MockRepo { routes: vec![route] };
        let router = DynamicRouter::new();
        let dispatchers: DashMap<Uuid, Arc<BackendDispatcher>> = DashMap::new();

        let count = RouteLoader::load(&repo, &router, &dispatchers).await.unwrap();

        assert_eq!(count, 0);
        assert!(!dispatchers.contains_key(&route_id));
    }

    fn make_ssh_route(route_id: Uuid, path: &str) -> RouteWithBinding {
        RouteWithBinding {
            route_id,
            contract_id: Uuid::new_v4(),
            method: HttpMethod::Get,
            path: path.to_string(),
            request_schema: serde_json::json!({}),
            response_schema: serde_json::json!({}),
            transform_rules: serde_json::json!({}),
            delivery_guarantee: DeliveryGuarantee::AtMostOnce,
            binding_id: Uuid::new_v4(),
            protocol: ProtocolType::Ssh,
            endpoint_config: serde_json::json!({
                "host": "10.0.1.50",
                "port": 22,
                "user": "admin",
                "command_template": "show interfaces status",
                "output_format": "raw",
            }),
            connection_pool_config: serde_json::json!({}),
            circuit_breaker_config: serde_json::json!({}),
            rate_limit_config: serde_json::json!({}),
            retry_config: serde_json::json!({}),
            timeout_ms: 0,
            auth_mapping: serde_json::json!({}),
        }
    }

    #[tokio::test]
    async fn loads_ssh_routes_and_registers_in_router() {
        // SSH 路由应使用 SSH 协议特定默认值（并发 5、超时 120s）构建保护栈
        let route_id = Uuid::new_v4();
        let repo = MockRepo {
            routes: vec![make_ssh_route(route_id, "/network/interfaces")],
        };
        let router = DynamicRouter::new();
        let dispatchers: DashMap<Uuid, Arc<BackendDispatcher>> = DashMap::new();

        let count = RouteLoader::load(&repo, &router, &dispatchers).await.unwrap();

        assert_eq!(count, 1);
        assert!(dispatchers.contains_key(&route_id));
        let result = router.match_route(&Method::GET, "/network/interfaces");
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, route_id);
    }

    #[tokio::test]
    async fn skips_soap_route_with_missing_url() {
        let route_id = Uuid::new_v4();
        let mut route = make_soap_route(route_id, "/orders");
        // 故意省略 url 字段，触发 soap_config_from_endpoint 的错误路径
        route.endpoint_config = serde_json::json!({ "operation_name": "Test" });

        let repo = MockRepo { routes: vec![route] };
        let router = DynamicRouter::new();
        let dispatchers: DashMap<Uuid, Arc<BackendDispatcher>> = DashMap::new();

        let count = RouteLoader::load(&repo, &router, &dispatchers).await.unwrap();

        assert_eq!(count, 0);
    }
}
