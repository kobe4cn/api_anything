use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 统一合约是所有 API 描述格式（WSDL、OpenAPI 等）转换后的中间表示，
/// 后续代码生成器以此为输入，避免各格式解析器直接耦合到生成逻辑
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedContract {
    pub service_name: String,
    pub description: String,
    pub base_path: String,
    pub operations: Vec<Operation>,
    pub types: Vec<TypeDef>,
}

/// 单个操作，统一映射到 HTTP 语义，SOAP 操作固定为 POST
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub name: String,
    pub description: String,
    pub http_method: String,
    pub path: String,
    pub input: Option<MessageDef>,
    pub output: Option<MessageDef>,
    /// 保留 SOAP action 供运行时代理层转发时使用
    pub soap_action: Option<String>,
    /// 原始服务端点，运行时代理需要知道实际调用地址
    pub endpoint_url: Option<String>,
}

/// 消息定义，schema 字段存储 JSON Schema 对象，
/// 统一用 serde_json::Value 以支持任意深度的嵌套结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageDef {
    pub name: String,
    pub schema: Value,
}

/// 可复用的类型定义，用于在多个操作间共享结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeDef {
    pub name: String,
    pub schema: Value,
}
