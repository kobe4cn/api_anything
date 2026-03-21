use crate::adapters::soap::{SoapAdapter, SoapConfig};
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

/// 从 RouteWithBinding 的 JSON 配置中提取连接池参数，
/// 不存在时返回合理的默认值，避免因配置缺失导致服务无法启动
fn extract_pool_config(config: &serde_json::Value) -> (u32, u32) {
    let max_concurrent = config
        .get("max_concurrent")
        .and_then(|v| v.as_u64())
        .unwrap_or(100) as u32;
    let rps = config
        .get("rps")
        .and_then(|v| v.as_u64())
        .unwrap_or(1000) as u32;
    (max_concurrent, rps)
}

/// 从 circuit_breaker_config 提取熔断参数；
/// error_threshold 为百分比（0-100），窗口/恢复时间单位为秒
fn extract_circuit_breaker_config(config: &serde_json::Value) -> (f64, Duration, Duration, u32) {
    let error_threshold = config
        .get("error_threshold_pct")
        .and_then(|v| v.as_f64())
        .unwrap_or(50.0);
    let window_secs = config
        .get("window_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(30);
    let open_secs = config
        .get("open_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(60);
    let half_open_max = config
        .get("half_open_max")
        .and_then(|v| v.as_u64())
        .unwrap_or(3) as u32;
    (
        error_threshold,
        Duration::from_secs(window_secs),
        Duration::from_secs(open_secs),
        half_open_max,
    )
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

pub struct RouteLoader;

impl RouteLoader {
    /// 从数据库加载所有启用的路由，构建 SoapAdapter + ProtectionStack + BackendDispatcher，
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
            // 目前只支持 SOAP，其他协议记录日志后跳过；
            // 未来增加协议支持时在此处扩展 match 分支
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
                ref other => {
                    tracing::warn!(
                        route_id = %route.route_id,
                        protocol = ?other,
                        "Skipping route: unsupported protocol"
                    );
                    continue;
                }
            };

            let (max_concurrent, rps) = extract_pool_config(&route.connection_pool_config);
            let (error_threshold, window_duration, open_duration, half_open_max) =
                extract_circuit_breaker_config(&route.circuit_breaker_config);

            // timeout_ms 存储为 i64 防止 SQL 类型溢出；实际超时不应为负，兜底为 30s
            let timeout = if route.timeout_ms > 0 {
                Duration::from_millis(route.timeout_ms as u64)
            } else {
                Duration::from_secs(30)
            };

            let protection = ProtectionStack::new(
                rps,
                max_concurrent,
                error_threshold,
                window_duration,
                open_duration,
                half_open_max,
                timeout,
            );

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
        async fn list_active_routes_with_bindings(&self) -> Result<Vec<RouteWithBinding>, AppError> {
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
