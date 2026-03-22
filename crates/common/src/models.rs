use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

// 枚举使用 sqlx::Type 以便直接映射到 PostgreSQL 的自定义类型，
// 避免在查询层做额外的字符串转换

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "source_type", rename_all = "snake_case")]
pub enum SourceType {
    Wsdl,
    Odata,
    Cli,
    Ssh,
    Pty,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "UPPERCASE")]
// sqlx 不支持 "uppercase"，使用 SCREAMING_SNAKE_CASE 映射到数据库存储的大写字符串
#[sqlx(type_name = "http_method", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "contract_status", rename_all = "snake_case")]
pub enum ContractStatus {
    Draft,
    Active,
    Deprecated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "protocol_type", rename_all = "snake_case")]
pub enum ProtocolType {
    Soap,
    Http,
    Cli,
    Ssh,
    Pty,
}

// AtMostOnce 是"发即忘"语义，最宽松的保证，适合作为默认值
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "delivery_guarantee", rename_all = "snake_case")]
pub enum DeliveryGuarantee {
    #[default]
    AtMostOnce,
    AtLeastOnce,
    ExactlyOnce,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "artifact_type", rename_all = "snake_case")]
pub enum ArtifactType {
    PluginSo,
    ConfigYaml,
    OpenapiJson,
    Dockerfile,
    TestSuite,
    AgentPrompt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "build_status", rename_all = "snake_case")]
pub enum BuildStatus {
    Building,
    Ready,
    Failed,
}

// Dead 表示已超出最大重试次数，不再尝试，等待人工干预
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "delivery_status", rename_all = "snake_case")]
pub enum DeliveryStatus {
    Pending,
    Delivered,
    Failed,
    Dead,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "sandbox_mode", rename_all = "snake_case")]
pub enum SandboxMode {
    Mock,
    Replay,
    Proxy,
}

// source_config 使用 JSON 存储不同来源类型（WSDL、OData 等）各异的连接参数，
// 避免为每种来源单独建表
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub owner: String,
    pub source_type: SourceType,
    pub source_config: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// parsed_model 保存解析后的中间表示，避免每次请求时重新解析原始 schema
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Contract {
    pub id: Uuid,
    pub project_id: Uuid,
    pub version: String,
    pub status: ContractStatus,
    pub original_schema: String,
    pub parsed_model: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// transform_rules 以 JSON 存储字段映射规则，支持在不修改代码的情况下
// 调整请求/响应的字段名、格式转换等行为
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Route {
    pub id: Uuid,
    pub contract_id: Uuid,
    pub method: HttpMethod,
    pub path: String,
    pub request_schema: Value,
    pub response_schema: Value,
    pub transform_rules: Value,
    pub backend_binding_id: Uuid,
    pub delivery_guarantee: DeliveryGuarantee,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// 各类配置（连接池、熔断、限流、重试）以 JSON 存储，
// 方便针对不同后端协议使用不同的配置 schema 而无需多张配置表
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct BackendBinding {
    pub id: Uuid,
    pub protocol: ProtocolType,
    pub endpoint_config: Value,
    pub connection_pool_config: Value,
    pub circuit_breaker_config: Value,
    pub rate_limit_config: Value,
    pub retry_config: Value,
    pub timeout_ms: i64,
    pub auth_mapping: Value,
}

// build_log 为可选，构建失败时才写入，避免正常情况下占用存储
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: Uuid,
    pub contract_id: Uuid,
    pub artifact_type: ArtifactType,
    pub content_hash: String,
    pub storage_path: String,
    pub build_status: BuildStatus,
    pub build_log: Option<String>,
    pub created_at: DateTime<Utc>,
}

// idempotency_key 允许客户端幂等重试；next_retry_at 由调度器决定指数退避时间
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryRecord {
    pub id: Uuid,
    pub route_id: Uuid,
    pub trace_id: String,
    pub idempotency_key: Option<String>,
    pub request_payload: Value,
    pub response_payload: Option<Value>,
    pub status: DeliveryStatus,
    pub retry_count: i32,
    pub next_retry_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// expires_at 用于自动清理过期沙箱会话，防止测试资源无限占用
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SandboxSession {
    pub id: Uuid,
    pub project_id: Uuid,
    pub tenant_id: String,
    pub mode: SandboxMode,
    pub config: Value,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedInteraction {
    pub id: Uuid,
    pub session_id: Uuid,
    pub route_id: Uuid,
    pub request: Value,
    pub response: Value,
    pub duration_ms: i32,
    pub recorded_at: DateTime<Utc>,
}

/// 路由 + 后端绑定的联合查询结果（网关启动时加载路由表用）
/// 通过一次 JOIN 查询避免 N+1 问题，减少网关启动时的数据库往返次数
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct RouteWithBinding {
    pub route_id: Uuid,
    pub contract_id: Uuid,
    #[sqlx(rename = "method")]
    pub method: HttpMethod,
    pub path: String,
    pub request_schema: serde_json::Value,
    pub response_schema: serde_json::Value,
    pub transform_rules: serde_json::Value,
    pub delivery_guarantee: DeliveryGuarantee,
    pub binding_id: Uuid,
    pub protocol: ProtocolType,
    pub endpoint_config: serde_json::Value,
    pub connection_pool_config: serde_json::Value,
    pub circuit_breaker_config: serde_json::Value,
    pub rate_limit_config: serde_json::Value,
    pub retry_config: serde_json::Value,
    pub timeout_ms: i64,
    pub auth_mapping: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_serialization_roundtrip() {
        let project = Project {
            id: uuid::Uuid::new_v4(),
            name: "legacy-soap-service".to_string(),
            description: "Legacy SOAP order service".to_string(),
            owner: "team-platform".to_string(),
            source_type: SourceType::Wsdl,
            source_config: serde_json::json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let json = serde_json::to_string(&project).unwrap();
        let deserialized: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(project.name, deserialized.name);
        assert_eq!(project.source_type, deserialized.source_type);
    }

    #[test]
    fn source_type_variants_serialize_as_lowercase() {
        assert_eq!(serde_json::to_string(&SourceType::Wsdl).unwrap(), "\"wsdl\"");
        assert_eq!(serde_json::to_string(&SourceType::Cli).unwrap(), "\"cli\"");
        assert_eq!(serde_json::to_string(&SourceType::Ssh).unwrap(), "\"ssh\"");
    }

    #[test]
    fn delivery_guarantee_default_is_at_most_once() {
        assert_eq!(DeliveryGuarantee::default(), DeliveryGuarantee::AtMostOnce);
    }

    #[test]
    fn http_method_includes_patch() {
        let method: HttpMethod = serde_json::from_str("\"PATCH\"").unwrap();
        assert_eq!(method, HttpMethod::Patch);
    }
}
