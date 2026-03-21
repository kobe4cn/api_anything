# Phase 1b: WSDL 解析 + SOAP 适配器 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现完整的 WSDL → REST API 自动转换管道：解析 WSDL 文件、生成统一契约模型、创建 SOAP 协议适配器、生成 OpenAPI 文档，并通过 CLI 命令端到端跑通。零 LLM 依赖，纯确定性映射。

**Architecture:** 新建 `generator` crate 封装生成引擎（WSDL 解析器 + 契约映射器 + OpenAPI 生成器）。在 `gateway` crate 中添加内置 SOAP ProtocolAdapter（配置驱动，从元数据读取端点和映射规则）。CLI `generate` 命令编排整个流水线：解析 WSDL → 生成契约 → 写入元数据 → 网关自动感知新路由。

**Tech Stack:** quick-xml (WSDL 解析), reqwest (SOAP HTTP 调用), serde_json (JSON Schema), 已有的 gateway + metadata crate

**Spec:** `docs/superpowers/specs/2026-03-21-api-anything-platform-design.md` §4.1-4.5

---

## File Structure

```
crates/generator/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── unified_contract.rs         # UnifiedContract 中间表示类型
    ├── wsdl/
    │   ├── mod.rs
    │   ├── parser.rs               # WSDL XML 结构化解析
    │   └── mapper.rs               # WSDL 解析结果 → UnifiedContract
    ├── openapi.rs                  # UnifiedContract → OpenAPI 3.0 JSON
    └── pipeline.rs                 # 编排 Stage 1→5 的流水线

crates/gateway/src/
    ├── adapters/
    │   ├── mod.rs
    │   └── soap.rs                 # 内置 SOAP ProtocolAdapter
    └── xml_json.rs                 # XML ↔ JSON 双向转换

crates/cli/src/
    └── main.rs                     # generate 子命令实现
```

同时修改:
- `Cargo.toml` (workspace) — 添加 generator 成员 + quick-xml 依赖
- `crates/metadata/src/repo.rs` — 扩展 MetadataRepo 增加写入契约/路由/绑定的方法
- `crates/metadata/src/pg.rs` — 实现写入方法
- `crates/platform-api/src/main.rs` — 启动时加载路由到 DynamicRouter

---

### Task 1: Generator Crate 脚手架 + UnifiedContract 类型

**Files:**
- Create: `crates/generator/Cargo.toml`
- Create: `crates/generator/src/lib.rs`
- Create: `crates/generator/src/unified_contract.rs`
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: 添加 generator 到 workspace**

在根 `Cargo.toml` 的 `members` 中添加 `"crates/generator"`，并添加 workspace 依赖：

```toml
quick-xml = { version = "0.37", features = ["serialize"] }
```

- [ ] **Step 2: 创建 crates/generator/Cargo.toml**

```toml
[package]
name = "api-anything-generator"
version.workspace = true
edition.workspace = true

[dependencies]
api-anything-common = { path = "../common" }
api-anything-metadata = { path = "../metadata" }
quick-xml.workspace = true
serde.workspace = true
serde_json.workspace = true
uuid.workspace = true
tracing.workspace = true
thiserror.workspace = true
anyhow.workspace = true

[dev-dependencies]
tokio.workspace = true
```

- [ ] **Step 3: 创建 unified_contract.rs — 统一中间表示**

这是所有输入（WSDL/CLI/SSH）解析后的统一数据结构，是生成引擎的核心数据模型。

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 统一契约模型 — 所有输入源解析后的标准中间表示
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedContract {
    pub service_name: String,
    pub description: String,
    pub base_path: String,
    pub operations: Vec<Operation>,
    pub types: Vec<TypeDef>,
}

/// 一个可调用的操作（对应一个 REST 端点）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub name: String,
    pub description: String,
    pub http_method: String,       // GET, POST, PUT, PATCH, DELETE
    pub path: String,              // /api/v1/orders/{id}
    pub input: Option<MessageDef>,
    pub output: Option<MessageDef>,
    pub soap_action: Option<String>,   // SOAP-specific: SOAPAction header
    pub endpoint_url: Option<String>,  // 后端服务地址
}

/// 消息定义（请求或响应的 body schema）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageDef {
    pub name: String,
    pub schema: Value,  // JSON Schema 格式
}

/// 类型定义（复杂类型、枚举等）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeDef {
    pub name: String,
    pub schema: Value,  // JSON Schema 格式
}
```

- [ ] **Step 4: 创建 lib.rs**

```rust
pub mod unified_contract;
pub mod wsdl;
pub mod openapi;
pub mod pipeline;
```

创建空的子模块文件占位。

- [ ] **Step 5: 验证编译 + Commit**

Run: `cargo check --workspace` (需设置 DATABASE_URL)

```bash
git commit -am "feat(generator): add generator crate with UnifiedContract types"
```

---

### Task 2: WSDL XML 解析器

**Files:**
- Create: `crates/generator/src/wsdl/mod.rs`
- Create: `crates/generator/src/wsdl/parser.rs`

WSDL 解析器从 XML 中提取结构化信息，不涉及 LLM。使用 quick-xml 的事件驱动或 serde 反序列化。

- [ ] **Step 1: 创建测试用 WSDL 文件**

创建 `crates/generator/tests/fixtures/calculator.wsdl`，一个简单但完整的 WSDL：

```xml
<?xml version="1.0" encoding="UTF-8"?>
<definitions name="CalculatorService"
  targetNamespace="http://example.com/calculator"
  xmlns="http://schemas.xmlsoap.org/wsdl/"
  xmlns:soap="http://schemas.xmlsoap.org/wsdl/soap/"
  xmlns:tns="http://example.com/calculator"
  xmlns:xsd="http://www.w3.org/2001/XMLSchema">

  <types>
    <xsd:schema targetNamespace="http://example.com/calculator">
      <xsd:element name="AddRequest">
        <xsd:complexType>
          <xsd:sequence>
            <xsd:element name="a" type="xsd:int"/>
            <xsd:element name="b" type="xsd:int"/>
          </xsd:sequence>
        </xsd:complexType>
      </xsd:element>
      <xsd:element name="AddResponse">
        <xsd:complexType>
          <xsd:sequence>
            <xsd:element name="result" type="xsd:int"/>
          </xsd:sequence>
        </xsd:complexType>
      </xsd:element>
      <xsd:element name="GetHistoryRequest">
        <xsd:complexType>
          <xsd:sequence>
            <xsd:element name="limit" type="xsd:int"/>
          </xsd:sequence>
        </xsd:complexType>
      </xsd:element>
      <xsd:element name="GetHistoryResponse">
        <xsd:complexType>
          <xsd:sequence>
            <xsd:element name="entries" type="xsd:string" maxOccurs="unbounded"/>
          </xsd:sequence>
        </xsd:complexType>
      </xsd:element>
    </xsd:schema>
  </types>

  <message name="AddInput"><part name="parameters" element="tns:AddRequest"/></message>
  <message name="AddOutput"><part name="parameters" element="tns:AddResponse"/></message>
  <message name="GetHistoryInput"><part name="parameters" element="tns:GetHistoryRequest"/></message>
  <message name="GetHistoryOutput"><part name="parameters" element="tns:GetHistoryResponse"/></message>

  <portType name="CalculatorPortType">
    <operation name="Add">
      <input message="tns:AddInput"/>
      <output message="tns:AddOutput"/>
    </operation>
    <operation name="GetHistory">
      <input message="tns:GetHistoryInput"/>
      <output message="tns:GetHistoryOutput"/>
    </operation>
  </portType>

  <binding name="CalculatorBinding" type="tns:CalculatorPortType">
    <soap:binding style="document" transport="http://schemas.xmlsoap.org/soap/http"/>
    <operation name="Add">
      <soap:operation soapAction="http://example.com/calculator/Add"/>
    </operation>
    <operation name="GetHistory">
      <soap:operation soapAction="http://example.com/calculator/GetHistory"/>
    </operation>
  </binding>

  <service name="CalculatorService">
    <port name="CalculatorPort" binding="tns:CalculatorBinding">
      <soap:address location="http://example.com/calculator"/>
    </port>
  </service>
</definitions>
```

- [ ] **Step 2: 编写解析器测试**

在 `parser.rs` 中写内联测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn sample_wsdl() -> &'static str {
        include_str!("../../tests/fixtures/calculator.wsdl")
    }

    #[test]
    fn parses_service_name() {
        let wsdl = WsdlParser::parse(sample_wsdl()).unwrap();
        assert_eq!(wsdl.service_name, "CalculatorService");
    }

    #[test]
    fn parses_operations() {
        let wsdl = WsdlParser::parse(sample_wsdl()).unwrap();
        assert_eq!(wsdl.operations.len(), 2);
        let add = wsdl.operations.iter().find(|o| o.name == "Add").unwrap();
        assert!(add.soap_action.as_ref().unwrap().contains("Add"));
    }

    #[test]
    fn parses_endpoint_url() {
        let wsdl = WsdlParser::parse(sample_wsdl()).unwrap();
        assert_eq!(wsdl.endpoint_url.as_deref(), Some("http://example.com/calculator"));
    }

    #[test]
    fn parses_types() {
        let wsdl = WsdlParser::parse(sample_wsdl()).unwrap();
        assert!(wsdl.types.len() >= 2); // AddRequest, AddResponse, etc.
    }

    #[test]
    fn parses_messages() {
        let wsdl = WsdlParser::parse(sample_wsdl()).unwrap();
        assert!(wsdl.messages.len() >= 4); // AddInput, AddOutput, etc.
    }
}
```

- [ ] **Step 3: 实现 WsdlParser**

定义 WSDL 解析的中间结构体（WsdlDefinition），使用 quick-xml 的 Reader 逐事件解析：

```rust
/// WSDL 解析的原始结构（未映射到 UnifiedContract）
#[derive(Debug, Clone)]
pub struct WsdlDefinition {
    pub service_name: String,
    pub target_namespace: String,
    pub endpoint_url: Option<String>,
    pub types: Vec<WsdlType>,
    pub messages: Vec<WsdlMessage>,
    pub operations: Vec<WsdlOperation>,
}

#[derive(Debug, Clone)]
pub struct WsdlType {
    pub name: String,
    pub elements: Vec<WsdlElement>,
}

#[derive(Debug, Clone)]
pub struct WsdlElement {
    pub name: String,
    pub type_name: String,     // xsd:int, xsd:string, etc.
    pub is_array: bool,        // maxOccurs="unbounded"
}

#[derive(Debug, Clone)]
pub struct WsdlMessage {
    pub name: String,
    pub element_ref: String,   // 引用的 type element 名
}

#[derive(Debug, Clone)]
pub struct WsdlOperation {
    pub name: String,
    pub input_message: String,
    pub output_message: String,
    pub soap_action: Option<String>,
}

pub struct WsdlParser;

impl WsdlParser {
    pub fn parse(xml: &str) -> Result<WsdlDefinition, anyhow::Error> {
        // 使用 quick-xml Reader 逐事件解析
        // 提取 <definitions>, <types>, <message>, <portType>, <binding>, <service>
        todo!("implement")
    }
}
```

实现要点：
- 使用 `quick_xml::Reader` 遍历事件
- 在 `<definitions>` 标签上提取 `name` 和 `targetNamespace`
- 在 `<types>/<xsd:schema>/<xsd:element>` 中解析类型定义
- 在 `<message>` 中解析消息和 part 引用
- 在 `<portType>/<operation>` 中解析操作和输入/输出消息引用
- 在 `<binding>/<operation>/<soap:operation>` 中提取 soapAction
- 在 `<service>/<port>/<soap:address>` 中提取 endpoint URL

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p api-anything-generator`
Expected: 5 tests PASS

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(generator): add WSDL XML parser with quick-xml"
```

---

### Task 3: WSDL → UnifiedContract 映射器

**Files:**
- Create: `crates/generator/src/wsdl/mapper.rs`

确定性规则将 WsdlDefinition 转换为 UnifiedContract。

- [ ] **Step 1: 编写映射器测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::wsdl::parser::WsdlParser;

    fn sample_wsdl() -> &'static str {
        include_str!("../../tests/fixtures/calculator.wsdl")
    }

    #[test]
    fn maps_service_name() {
        let wsdl = WsdlParser::parse(sample_wsdl()).unwrap();
        let contract = WsdlMapper::map(&wsdl).unwrap();
        assert_eq!(contract.service_name, "CalculatorService");
    }

    #[test]
    fn maps_operations_to_rest_routes() {
        let wsdl = WsdlParser::parse(sample_wsdl()).unwrap();
        let contract = WsdlMapper::map(&wsdl).unwrap();
        assert_eq!(contract.operations.len(), 2);

        let add = contract.operations.iter().find(|o| o.name == "Add").unwrap();
        assert_eq!(add.http_method, "POST");  // SOAP 操作默认映射为 POST
        assert!(add.path.contains("add"));    // 路径名小写化
        assert!(add.soap_action.is_some());
    }

    #[test]
    fn generates_json_schema_for_input() {
        let wsdl = WsdlParser::parse(sample_wsdl()).unwrap();
        let contract = WsdlMapper::map(&wsdl).unwrap();
        let add = contract.operations.iter().find(|o| o.name == "Add").unwrap();
        let input = add.input.as_ref().unwrap();
        // schema 应包含 a 和 b 两个 integer 字段
        let props = input.schema.get("properties").unwrap();
        assert!(props.get("a").is_some());
        assert!(props.get("b").is_some());
    }

    #[test]
    fn maps_xsd_types_to_json_schema_types() {
        let wsdl = WsdlParser::parse(sample_wsdl()).unwrap();
        let contract = WsdlMapper::map(&wsdl).unwrap();
        let add = contract.operations.iter().find(|o| o.name == "Add").unwrap();
        let input = add.input.as_ref().unwrap();
        let a_type = input.schema["properties"]["a"]["type"].as_str().unwrap();
        assert_eq!(a_type, "integer");  // xsd:int → integer
    }

    #[test]
    fn preserves_endpoint_url() {
        let wsdl = WsdlParser::parse(sample_wsdl()).unwrap();
        let contract = WsdlMapper::map(&wsdl).unwrap();
        let add = contract.operations.iter().find(|o| o.name == "Add").unwrap();
        assert_eq!(add.endpoint_url.as_deref(), Some("http://example.com/calculator"));
    }
}
```

- [ ] **Step 2: 实现 WsdlMapper**

```rust
use crate::unified_contract::*;
use crate::wsdl::parser::*;
use serde_json::json;

pub struct WsdlMapper;

impl WsdlMapper {
    pub fn map(wsdl: &WsdlDefinition) -> Result<UnifiedContract, anyhow::Error> {
        let base_path = format!("/api/v1/{}", to_kebab_case(&wsdl.service_name));
        let mut operations = Vec::new();

        for op in &wsdl.operations {
            let input_schema = Self::resolve_message_schema(
                &op.input_message, &wsdl.messages, &wsdl.types
            );
            let output_schema = Self::resolve_message_schema(
                &op.output_message, &wsdl.messages, &wsdl.types
            );

            operations.push(Operation {
                name: op.name.clone(),
                description: format!("SOAP operation: {}", op.name),
                http_method: "POST".to_string(), // SOAP 操作默认 POST
                path: format!("{}/{}", base_path, to_kebab_case(&op.name)),
                input: input_schema.map(|s| MessageDef {
                    name: format!("{}Request", op.name),
                    schema: s,
                }),
                output: output_schema.map(|s| MessageDef {
                    name: format!("{}Response", op.name),
                    schema: s,
                }),
                soap_action: op.soap_action.clone(),
                endpoint_url: wsdl.endpoint_url.clone(),
            });
        }

        // 收集所有类型定义
        let types = wsdl.types.iter().map(|t| {
            TypeDef {
                name: t.name.clone(),
                schema: Self::type_to_json_schema(t),
            }
        }).collect();

        Ok(UnifiedContract {
            service_name: wsdl.service_name.clone(),
            description: format!("Auto-generated from WSDL: {}", wsdl.service_name),
            base_path,
            operations,
            types,
        })
    }

    fn resolve_message_schema(
        message_name: &str,
        messages: &[WsdlMessage],
        types: &[WsdlType],
    ) -> Option<serde_json::Value> {
        let msg = messages.iter().find(|m| m.name == message_name)?;
        let type_def = types.iter().find(|t| t.name == msg.element_ref)?;
        Some(Self::type_to_json_schema(type_def))
    }

    fn type_to_json_schema(t: &WsdlType) -> serde_json::Value {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();

        for elem in &t.elements {
            let json_type = Self::xsd_to_json_type(&elem.type_name);
            let prop = if elem.is_array {
                json!({ "type": "array", "items": { "type": json_type } })
            } else {
                json!({ "type": json_type })
            };
            properties.insert(elem.name.clone(), prop);
            required.push(serde_json::Value::String(elem.name.clone()));
        }

        json!({
            "type": "object",
            "properties": properties,
            "required": required
        })
    }

    /// XSD 基本类型 → JSON Schema 类型映射
    fn xsd_to_json_type(xsd_type: &str) -> &str {
        // 去掉 namespace 前缀
        let local = xsd_type.split(':').last().unwrap_or(xsd_type);
        match local {
            "int" | "integer" | "long" | "short" | "byte" => "integer",
            "float" | "double" | "decimal" => "number",
            "boolean" => "boolean",
            "string" | "dateTime" | "date" | "time" | "anyURI" => "string",
            _ => "string", // 未知类型默认 string
        }
    }
}

/// 驼峰/Pascal 转 kebab-case
fn to_kebab_case(s: &str) -> String {
    let mut result = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            result.push('-');
        }
        result.push(c.to_ascii_lowercase());
    }
    // 移除 -service 后缀
    result.trim_end_matches("-service").to_string()
}
```

- [ ] **Step 3: 更新 wsdl/mod.rs**

```rust
pub mod parser;
pub mod mapper;
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p api-anything-generator`
Expected: parser 5 + mapper 5 = 10 tests PASS

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(generator): add WSDL to UnifiedContract mapper with XSD→JSON Schema conversion"
```

---

### Task 4: XML ↔ JSON 双向转换器

**Files:**
- Create: `crates/gateway/src/xml_json.rs`

SOAP 适配器需要将 JSON 请求转为 SOAP XML，并将 SOAP XML 响应转回 JSON。

- [ ] **Step 1: 编写转换器测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_to_soap_envelope() {
        let json = serde_json::json!({ "a": 1, "b": 2 });
        let xml = SoapXmlBuilder::build_envelope(
            "http://example.com/calculator/Add",
            "Add",
            "http://example.com/calculator",
            &json,
        );
        assert!(xml.contains("<soap:Envelope"));
        assert!(xml.contains("<soap:Body>"));
        assert!(xml.contains("<a>1</a>"));
        assert!(xml.contains("<b>2</b>"));
        assert!(xml.contains("Add"));  // 操作名包含在 Body 中
    }

    #[test]
    fn soap_response_to_json() {
        let xml = r#"<?xml version="1.0"?>
        <soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body>
            <AddResponse xmlns="http://example.com/calculator">
              <result>3</result>
            </AddResponse>
          </soap:Body>
        </soap:Envelope>"#;

        let json = SoapXmlParser::parse_response(xml).unwrap();
        assert_eq!(json["result"], "3");
    }

    #[test]
    fn handles_nested_elements() {
        let xml = r#"<?xml version="1.0"?>
        <soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body>
            <Response xmlns="http://example.com">
              <order>
                <id>123</id>
                <status>active</status>
              </order>
            </Response>
          </soap:Body>
        </soap:Envelope>"#;

        let json = SoapXmlParser::parse_response(xml).unwrap();
        assert_eq!(json["order"]["id"], "123");
    }

    #[test]
    fn handles_empty_response_body() {
        let xml = r#"<?xml version="1.0"?>
        <soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body/>
        </soap:Envelope>"#;

        let json = SoapXmlParser::parse_response(xml).unwrap();
        assert!(json.is_object());
    }
}
```

- [ ] **Step 2: 实现 SoapXmlBuilder + SoapXmlParser**

```rust
use quick_xml::events::{Event, BytesStart, BytesEnd, BytesText};
use quick_xml::{Reader, Writer};
use serde_json::Value;
use std::io::Cursor;

pub struct SoapXmlBuilder;

impl SoapXmlBuilder {
    /// 将 JSON 请求体包装为 SOAP Envelope
    pub fn build_envelope(
        soap_action: &str,
        operation_name: &str,
        namespace: &str,
        body: &Value,
    ) -> String {
        let mut xml = String::new();
        xml.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
        xml.push_str(r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/" xmlns:ns=""#);
        xml.push_str(namespace);
        xml.push_str(r#"">"#);
        xml.push_str("<soap:Body>");
        xml.push_str(&format!("<ns:{}>", operation_name));

        // 将 JSON 字段转为 XML 元素
        if let Value::Object(map) = body {
            for (key, val) in map {
                Self::value_to_xml(&mut xml, key, val);
            }
        }

        xml.push_str(&format!("</ns:{}>", operation_name));
        xml.push_str("</soap:Body>");
        xml.push_str("</soap:Envelope>");
        xml
    }

    fn value_to_xml(xml: &mut String, key: &str, val: &Value) {
        match val {
            Value::Object(map) => {
                xml.push_str(&format!("<{}>", key));
                for (k, v) in map {
                    Self::value_to_xml(xml, k, v);
                }
                xml.push_str(&format!("</{}>", key));
            }
            Value::Array(arr) => {
                for item in arr {
                    Self::value_to_xml(xml, key, item);
                }
            }
            _ => {
                xml.push_str(&format!("<{}>{}</{}>", key, val_to_string(val), key));
            }
        }
    }
}

fn val_to_string(val: &Value) -> String {
    match val {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        _ => val.to_string(),
    }
}

pub struct SoapXmlParser;

impl SoapXmlParser {
    /// 从 SOAP Envelope 响应中提取 Body 内容转为 JSON
    pub fn parse_response(xml: &str) -> Result<Value, anyhow::Error> {
        // 使用 quick-xml Reader 逐事件解析
        // 跳过 Envelope 和 Body 标签，提取 Body 内第一个子元素的内容
        let mut reader = Reader::from_str(xml);
        let mut in_body = false;
        let mut body_depth = 0;
        let mut result = serde_json::Map::new();
        let mut stack: Vec<(String, serde_json::Map<String, Value>)> = Vec::new();

        // 简化实现：找到 Body 内的内容，递归转为 JSON
        let body_content = Self::extract_body_content(xml)?;
        Self::xml_element_to_json(&body_content)
    }

    fn extract_body_content(xml: &str) -> Result<String, anyhow::Error> {
        // 找到 <soap:Body> 和 </soap:Body> 之间的内容
        let body_start = xml.find("<soap:Body>")
            .or_else(|| xml.find("<soap:Body/>"))
            .ok_or_else(|| anyhow::anyhow!("No soap:Body found"))?;

        if xml[body_start..].starts_with("<soap:Body/>") {
            return Ok(String::new());
        }

        let content_start = body_start + "<soap:Body>".len();
        let content_end = xml.find("</soap:Body>")
            .ok_or_else(|| anyhow::anyhow!("No closing soap:Body"))?;
        Ok(xml[content_start..content_end].trim().to_string())
    }

    fn xml_element_to_json(xml: &str) -> Result<Value, anyhow::Error> {
        if xml.is_empty() {
            return Ok(Value::Object(serde_json::Map::new()));
        }

        let mut reader = Reader::from_str(xml);
        let mut result = serde_json::Map::new();
        let mut current_key = String::new();
        let mut current_text = String::new();
        let mut depth = 0;
        let mut nested_xml = String::new();
        let mut collecting_nested = false;
        let mut nested_depth = 0;
        let mut root_skipped = false;

        // 简化：跳过根元素（如 <AddResponse>），提取其子元素
        // 递归处理嵌套元素
        Self::parse_children_to_map(xml, &mut result)?;
        Ok(Value::Object(result))
    }

    fn parse_children_to_map(xml: &str, map: &mut serde_json::Map<String, Value>) -> Result<(), anyhow::Error> {
        let mut reader = Reader::from_str(xml);
        let mut buf = Vec::new();
        let mut depth = 0;
        let mut current_key = String::new();
        let mut skip_root = true;

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) => {
                    let name = Self::local_name(e.name().as_ref());
                    depth += 1;
                    if skip_root && depth == 1 {
                        // 跳过根元素
                        skip_root = false;
                        continue;
                    }
                    current_key = name;
                }
                Ok(Event::Text(e)) => {
                    if !current_key.is_empty() && depth >= 1 {
                        let text = e.unescape()?.to_string();
                        if !text.trim().is_empty() {
                            map.insert(current_key.clone(), Value::String(text.trim().to_string()));
                        }
                    }
                }
                Ok(Event::End(_)) => {
                    depth -= 1;
                    if depth <= 0 { break; }
                    current_key.clear();
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(anyhow::anyhow!("XML parse error: {}", e)),
                _ => {}
            }
            buf.clear();
        }
        Ok(())
    }

    fn local_name(full_name: &[u8]) -> String {
        let s = std::str::from_utf8(full_name).unwrap_or("");
        s.split(':').last().unwrap_or(s).to_string()
    }
}
```

注意：这是一个简化的实现，处理平坦和单层嵌套的 SOAP 响应。深度嵌套需要递归处理，可在后续迭代中增强。

- [ ] **Step 3: 更新 gateway lib.rs + Cargo.toml**

在 gateway 的 Cargo.toml 添加 `quick-xml.workspace = true` 和 `anyhow.workspace = true`。
在 lib.rs 添加 `pub mod xml_json;`

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p api-anything-gateway xml_json`
Expected: 4 tests PASS

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(gateway): add SOAP XML↔JSON bidirectional converter"
```

---

### Task 5: 内置 SOAP ProtocolAdapter

**Files:**
- Create: `crates/gateway/src/adapters/mod.rs`
- Create: `crates/gateway/src/adapters/soap.rs`

配置驱动的 SOAP 适配器，从元数据读取端点和 SOAP Action。

- [ ] **Step 1: 编写 SOAP 适配器测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_request_builds_soap_envelope() {
        let adapter = SoapAdapter::new(SoapConfig {
            endpoint_url: "http://example.com/calculator".to_string(),
            soap_action: "http://example.com/calculator/Add".to_string(),
            operation_name: "Add".to_string(),
            namespace: "http://example.com/calculator".to_string(),
        });

        let req = GatewayRequest {
            route_id: Uuid::new_v4(),
            method: Method::POST,
            path: "/api/v1/calculator/add".to_string(),
            headers: HeaderMap::new(),
            query_params: HashMap::new(),
            path_params: HashMap::new(),
            body: Some(serde_json::json!({"a": 1, "b": 2})),
            trace_id: "test".to_string(),
        };

        let backend_req = adapter.transform_request(&req).unwrap();
        assert_eq!(backend_req.endpoint, "http://example.com/calculator");
        let body = String::from_utf8(backend_req.body.unwrap()).unwrap();
        assert!(body.contains("<soap:Envelope"));
        assert!(body.contains("<a>1</a>"));
    }

    #[test]
    fn transform_response_parses_soap_xml() {
        let adapter = SoapAdapter::new(SoapConfig {
            endpoint_url: "http://example.com".to_string(),
            soap_action: "test".to_string(),
            operation_name: "Add".to_string(),
            namespace: "http://example.com".to_string(),
        });

        let soap_response = r#"<?xml version="1.0"?>
        <soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body><AddResponse xmlns="http://example.com"><result>42</result></AddResponse></soap:Body>
        </soap:Envelope>"#;

        let backend_resp = BackendResponse {
            status_code: 200,
            headers: HeaderMap::new(),
            body: soap_response.as_bytes().to_vec(),
            is_success: true,
            duration_ms: 50,
        };

        let resp = adapter.transform_response(&backend_resp).unwrap();
        assert_eq!(resp.status_code, 200);
        assert_eq!(resp.body["result"], "42");
    }
}
```

- [ ] **Step 2: 实现 SoapAdapter**

```rust
use crate::adapter::ProtocolAdapter;
use crate::types::*;
use crate::xml_json::{SoapXmlBuilder, SoapXmlParser};
use api_anything_common::error::AppError;
use axum::http::{HeaderMap, HeaderValue, Method};
use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct SoapConfig {
    pub endpoint_url: String,
    pub soap_action: String,
    pub operation_name: String,
    pub namespace: String,
}

pub struct SoapAdapter {
    config: SoapConfig,
    client: reqwest::Client,
}

impl SoapAdapter {
    pub fn new(config: SoapConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }
}

impl ProtocolAdapter for SoapAdapter {
    fn transform_request(&self, req: &GatewayRequest) -> Result<BackendRequest, AppError> {
        let body = req.body.as_ref().cloned().unwrap_or(serde_json::Value::Object(Default::default()));
        let xml = SoapXmlBuilder::build_envelope(
            &self.config.soap_action,
            &self.config.operation_name,
            &self.config.namespace,
            &body,
        );

        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", HeaderValue::from_static("text/xml; charset=utf-8"));
        headers.insert("SOAPAction", HeaderValue::from_str(&self.config.soap_action)
            .map_err(|e| AppError::Internal(format!("Invalid SOAPAction: {e}")))?);

        let mut protocol_params = HashMap::new();
        protocol_params.insert("soap_action".to_string(), self.config.soap_action.clone());

        Ok(BackendRequest {
            endpoint: self.config.endpoint_url.clone(),
            method: Method::POST,
            headers,
            body: Some(xml.into_bytes()),
            protocol_params,
        })
    }

    fn execute<'a>(&'a self, req: &'a BackendRequest) -> BoxFuture<'a, Result<BackendResponse, AppError>> {
        Box::pin(async move {
            let start = Instant::now();
            let response = self.client
                .post(&req.endpoint)
                .headers(req.headers.clone())
                .body(req.body.clone().unwrap_or_default())
                .send()
                .await
                .map_err(|e| AppError::BackendUnavailable(format!("SOAP request failed: {e}")))?;

            let status = response.status().as_u16();
            let headers = response.headers().clone();
            let body = response.bytes().await
                .map_err(|e| AppError::Internal(format!("Failed to read response: {e}")))?;

            let is_success = status < 400 && !body.iter().any(|_| false); // 简化：后续 ErrorNormalizer 会做详细检查
            Ok(BackendResponse {
                status_code: status,
                headers,
                body: body.to_vec(),
                is_success: status < 400,
                duration_ms: start.elapsed().as_millis() as u64,
            })
        })
    }

    fn transform_response(&self, resp: &BackendResponse) -> Result<GatewayResponse, AppError> {
        let body_text = String::from_utf8_lossy(&resp.body);
        let json = SoapXmlParser::parse_response(&body_text)
            .map_err(|e| AppError::Internal(format!("Failed to parse SOAP response: {e}")))?;

        Ok(GatewayResponse {
            status_code: resp.status_code,
            headers: HashMap::new(),
            body: json,
        })
    }

    fn name(&self) -> &str {
        "soap"
    }
}
```

注意：`execute` 方法的签名需要与 adapter.rs 中 ProtocolAdapter trait 定义的 BoxFuture 签名一致。检查当前 trait 定义并匹配。

- [ ] **Step 3: 更新 gateway lib.rs**

添加 `pub mod adapters;`

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p api-anything-gateway`
Expected: 所有测试通过（24 protection + 4 router + 4 xml_json + 2 soap = ~34）

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(gateway): add built-in SOAP protocol adapter"
```

---

### Task 6: OpenAPI 3.0 生成器

**Files:**
- Create: `crates/generator/src/openapi.rs`

从 UnifiedContract 生成标准 OpenAPI 3.0 JSON。

- [ ] **Step 1: 编写 OpenAPI 生成器测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::wsdl::{parser::WsdlParser, mapper::WsdlMapper};

    fn sample_contract() -> UnifiedContract {
        let wsdl = WsdlParser::parse(include_str!("../tests/fixtures/calculator.wsdl")).unwrap();
        WsdlMapper::map(&wsdl).unwrap()
    }

    #[test]
    fn generates_valid_openapi_structure() {
        let contract = sample_contract();
        let spec = OpenApiGenerator::generate(&contract);
        assert_eq!(spec["openapi"], "3.0.3");
        assert!(spec["info"]["title"].as_str().unwrap().contains("Calculator"));
        assert!(spec["paths"].is_object());
    }

    #[test]
    fn generates_paths_for_operations() {
        let contract = sample_contract();
        let spec = OpenApiGenerator::generate(&contract);
        let paths = spec["paths"].as_object().unwrap();
        assert_eq!(paths.len(), 2); // Add + GetHistory
    }

    #[test]
    fn includes_request_body_schema() {
        let contract = sample_contract();
        let spec = OpenApiGenerator::generate(&contract);
        let paths = spec["paths"].as_object().unwrap();
        // 找到一个 path 并检查 requestBody
        let (_, path_item) = paths.iter().next().unwrap();
        let post = &path_item["post"];
        assert!(post["requestBody"].is_object());
    }

    #[test]
    fn includes_response_schema() {
        let contract = sample_contract();
        let spec = OpenApiGenerator::generate(&contract);
        let paths = spec["paths"].as_object().unwrap();
        let (_, path_item) = paths.iter().next().unwrap();
        let post = &path_item["post"];
        assert!(post["responses"]["200"].is_object());
    }
}
```

- [ ] **Step 2: 实现 OpenApiGenerator**

```rust
use crate::unified_contract::*;
use serde_json::{json, Value};

pub struct OpenApiGenerator;

impl OpenApiGenerator {
    pub fn generate(contract: &UnifiedContract) -> Value {
        let mut paths = serde_json::Map::new();

        for op in &contract.operations {
            let method = op.http_method.to_lowercase();
            let mut operation = serde_json::Map::new();

            operation.insert("operationId".to_string(), Value::String(op.name.clone()));
            operation.insert("summary".to_string(), Value::String(op.description.clone()));
            operation.insert("tags".to_string(), json!([contract.service_name]));

            // Request body
            if let Some(input) = &op.input {
                operation.insert("requestBody".to_string(), json!({
                    "required": true,
                    "content": {
                        "application/json": {
                            "schema": input.schema
                        }
                    }
                }));
            }

            // Responses
            let mut responses = serde_json::Map::new();
            if let Some(output) = &op.output {
                responses.insert("200".to_string(), json!({
                    "description": "Successful response",
                    "content": {
                        "application/json": {
                            "schema": output.schema
                        }
                    }
                }));
            } else {
                responses.insert("200".to_string(), json!({
                    "description": "Successful response"
                }));
            }
            responses.insert("429".to_string(), json!({ "description": "Rate limited" }));
            responses.insert("502".to_string(), json!({ "description": "Backend error" }));
            responses.insert("503".to_string(), json!({ "description": "Circuit breaker open" }));
            responses.insert("504".to_string(), json!({ "description": "Backend timeout" }));

            operation.insert("responses".to_string(), Value::Object(responses));

            let mut method_map = serde_json::Map::new();
            method_map.insert(method, Value::Object(operation));
            paths.insert(op.path.clone(), Value::Object(method_map));
        }

        json!({
            "openapi": "3.0.3",
            "info": {
                "title": format!("{} API", contract.service_name),
                "description": contract.description,
                "version": "1.0.0"
            },
            "paths": paths
        })
    }
}
```

- [ ] **Step 3: 运行测试确认通过**

Run: `cargo test -p api-anything-generator`
Expected: parser 5 + mapper 5 + openapi 4 = 14 tests PASS

- [ ] **Step 4: Commit**

```bash
git commit -am "feat(generator): add OpenAPI 3.0 generator from UnifiedContract"
```

---

### Task 7: 扩展 MetadataRepo — 写入契约/路由/绑定

**Files:**
- Modify: `crates/metadata/src/repo.rs`
- Modify: `crates/metadata/src/pg.rs`

生成流水线需要将解析结果写入元数据仓库。

- [ ] **Step 1: 扩展 MetadataRepo trait**

```rust
// 新增方法：
async fn create_contract(&self, project_id: Uuid, version: &str, original_schema: &str, parsed_model: &Value) -> Result<Contract, AppError>;
async fn create_backend_binding(&self, protocol: ProtocolType, endpoint_config: &Value, timeout_ms: i64) -> Result<BackendBinding, AppError>;
async fn create_route(&self, contract_id: Uuid, method: HttpMethod, path: &str, request_schema: &Value, response_schema: &Value, transform_rules: &Value, backend_binding_id: Uuid) -> Result<Route, AppError>;
```

- [ ] **Step 2: 实现 PG 版本**

使用 `INSERT ... RETURNING` 返回完整记录。

- [ ] **Step 3: 验证编译 + Commit**

```bash
git commit -am "feat(metadata): add create contract, binding, and route methods"
```

---

### Task 8: 生成流水线编排 + CLI 集成

**Files:**
- Create: `crates/generator/src/pipeline.rs`
- Modify: `crates/cli/Cargo.toml` + `src/main.rs`

- [ ] **Step 1: 实现 Pipeline**

```rust
use crate::unified_contract::UnifiedContract;
use crate::wsdl::{parser::WsdlParser, mapper::WsdlMapper};
use crate::openapi::OpenApiGenerator;
use api_anything_common::models::*;
use api_anything_metadata::MetadataRepo;
use serde_json::Value;
use uuid::Uuid;

pub struct GenerationPipeline;

impl GenerationPipeline {
    /// 执行 WSDL → REST 全流水线
    pub async fn run_wsdl(
        repo: &impl MetadataRepo,
        project_id: Uuid,
        wsdl_content: &str,
    ) -> Result<GenerationResult, anyhow::Error> {
        // Stage 1: 解析 WSDL
        tracing::info!("Stage 1: Parsing WSDL");
        let wsdl = WsdlParser::parse(wsdl_content)?;

        // Stage 2: 映射为 UnifiedContract
        tracing::info!("Stage 2: Mapping to UnifiedContract");
        let contract = WsdlMapper::map(&wsdl)?;

        // Stage 3: 写入元数据
        tracing::info!("Stage 3: Persisting to metadata");
        let db_contract = repo.create_contract(
            project_id,
            "1.0.0",
            wsdl_content,
            &serde_json::to_value(&contract)?,
        ).await?;

        let mut routes = Vec::new();
        for op in &contract.operations {
            // 创建 BackendBinding
            let endpoint_config = serde_json::json!({
                "url": op.endpoint_url,
                "soap_action": op.soap_action,
                "operation_name": op.name,
                "namespace": wsdl.target_namespace,
            });
            let binding = repo.create_backend_binding(
                ProtocolType::Soap,
                &endpoint_config,
                30000, // 默认 30s
            ).await?;

            // 创建 Route
            let method = match op.http_method.as_str() {
                "GET" => HttpMethod::Get,
                "PUT" => HttpMethod::Put,
                "DELETE" => HttpMethod::Delete,
                "PATCH" => HttpMethod::Patch,
                _ => HttpMethod::Post,
            };
            let request_schema = op.input.as_ref().map(|m| m.schema.clone()).unwrap_or(serde_json::json!({}));
            let response_schema = op.output.as_ref().map(|m| m.schema.clone()).unwrap_or(serde_json::json!({}));

            let route = repo.create_route(
                db_contract.id,
                method,
                &op.path,
                &request_schema,
                &response_schema,
                &endpoint_config, // transform_rules 暂存 endpoint config
                binding.id,
            ).await?;
            routes.push(route);
        }

        // Stage 5: 生成 OpenAPI
        tracing::info!("Stage 5: Generating OpenAPI spec");
        let openapi = OpenApiGenerator::generate(&contract);

        Ok(GenerationResult {
            contract_id: db_contract.id,
            routes_count: routes.len(),
            openapi_spec: openapi,
        })
    }
}

#[derive(Debug)]
pub struct GenerationResult {
    pub contract_id: Uuid,
    pub routes_count: usize,
    pub openapi_spec: Value,
}
```

- [ ] **Step 2: 更新 CLI generate 命令**

```rust
use clap::Parser;
use api_anything_common::config::AppConfig;
use api_anything_metadata::PgMetadataRepo;
use api_anything_metadata::MetadataRepo;
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
    /// Generate REST API from legacy system contract
    Generate {
        /// Path to source contract (WSDL file)
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
            repo.run_migrations().await?;

            // 读取 WSDL 文件
            let wsdl_content = std::fs::read_to_string(&source)?;

            // 创建或查找项目
            let project_obj = repo.create_project(
                &project, "Auto-generated project", "cli",
                api_anything_common::models::SourceType::Wsdl,
            ).await?;

            // 运行流水线
            let result = GenerationPipeline::run_wsdl(&repo, project_obj.id, &wsdl_content).await?;

            println!("Generation complete!");
            println!("  Contract ID: {}", result.contract_id);
            println!("  Routes created: {}", result.routes_count);

            // 输出 OpenAPI spec
            let spec_path = format!("{}.openapi.json", source);
            std::fs::write(&spec_path, serde_json::to_string_pretty(&result.openapi_spec)?)?;
            println!("  OpenAPI spec: {}", spec_path);
        }
    }

    Ok(())
}
```

CLI 的 Cargo.toml 需要添加依赖：
```toml
api-anything-metadata = { path = "../metadata" }
api-anything-generator = { path = "../generator" }
sqlx.workspace = true
dotenvy.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
```

- [ ] **Step 3: 验证编译**

Run: `DATABASE_URL=postgres://api_anything:api_anything@localhost:5432/api_anything cargo check --workspace`

- [ ] **Step 4: Commit**

```bash
git commit -am "feat: add generation pipeline with CLI generate command"
```

---

### Task 9: 端到端验证 — WSDL → REST API

**Files:** 无新文件

- [ ] **Step 1: 运行 CLI 生成**

```bash
DATABASE_URL=postgres://api_anything:api_anything@localhost:5432/api_anything \
cargo run -p api-anything-cli -- generate \
  --source crates/generator/tests/fixtures/calculator.wsdl \
  --project calculator-service
```

Expected: 生成成功，输出 Contract ID、Routes count、OpenAPI spec 路径

- [ ] **Step 2: 验证元数据写入**

```bash
podman exec docker-postgres-1 psql -U api_anything -d api_anything -c "SELECT id, name, source_type FROM projects WHERE name = 'calculator-service';"
podman exec docker-postgres-1 psql -U api_anything -d api_anything -c "SELECT r.method, r.path, r.enabled FROM routes r JOIN contracts c ON r.contract_id = c.id JOIN projects p ON c.project_id = p.id WHERE p.name = 'calculator-service';"
```

Expected: 项目记录存在，2 条路由（Add + GetHistory）均为 enabled=true

- [ ] **Step 3: 验证 OpenAPI spec**

检查生成的 `.openapi.json` 文件：
- 包含 `openapi: 3.0.3`
- 包含 2 个 paths
- 每个 path 有 requestBody 和 responses

- [ ] **Step 4: 运行全量测试**

Run: `DATABASE_URL=postgres://api_anything:api_anything@localhost:5432/api_anything cargo test --workspace`
Expected: 所有测试通过

- [ ] **Step 5: Commit（如有修复）**

```bash
git commit -am "fix: address issues found during Phase 1b e2e validation"
```

---

## Summary

| Task | 组件 | 产出 |
|------|------|------|
| 1 | Generator Crate | UnifiedContract 中间表示类型 |
| 2 | WSDL Parser | quick-xml 结构化解析 + 5 测试 |
| 3 | WSDL Mapper | WSDL → UnifiedContract + XSD→JSON Schema + 5 测试 |
| 4 | XML↔JSON | SOAP Envelope 构建/解析 + 4 测试 |
| 5 | SOAP Adapter | 内置 ProtocolAdapter + 2 测试 |
| 6 | OpenAPI Gen | UnifiedContract → OpenAPI 3.0 + 4 测试 |
| 7 | Metadata 扩展 | 写入契约/路由/绑定方法 |
| 8 | Pipeline + CLI | Stage 1-5 编排 + CLI generate 命令 |
| 9 | E2E 验证 | WSDL → DB → OpenAPI 全链路 |

**Phase 1b 验收标准：** 使用 `calculator.wsdl` 作为输入，CLI `generate` 命令能完成全链路：解析 WSDL → 生成 UnifiedContract → 写入元数据（项目+契约+路由+绑定）→ 生成 OpenAPI 3.0 spec 文件。数据库中可查到启用的路由记录。

**不在 Phase 1b 范围内：**
- LLM 增强的语义映射（Phase 1c）
- 网关运行时自动加载新生成的路由（需要 Phase 1a 的轮询机制，在集成阶段补充）
- 影子测试生成、Agent 提示词生成（Phase 1c）
- .so 插件编译（后续阶段）
