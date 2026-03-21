use anyhow::{Context, Result};
use quick_xml::{
    events::{BytesStart, Event},
    Reader,
};

/// WSDL 文档的完整解析结果，只保留代码生成所需的语义信息
#[derive(Debug, Clone)]
pub struct WsdlDefinition {
    pub service_name: String,
    pub target_namespace: String,
    pub endpoint_url: Option<String>,
    pub types: Vec<WsdlType>,
    pub messages: Vec<WsdlMessage>,
    pub operations: Vec<WsdlOperation>,
}

/// XSD 顶层 element，对应一个请求/响应结构体
#[derive(Debug, Clone)]
pub struct WsdlType {
    pub name: String,
    pub elements: Vec<WsdlElement>,
}

/// XSD sequence 中的字段
#[derive(Debug, Clone)]
pub struct WsdlElement {
    pub name: String,
    /// 去掉命名空间前缀后的原始 XSD 类型名，如 "int"、"string"
    pub type_name: String,
    /// maxOccurs="unbounded" 标记为数组，生成 JSON Schema 时映射为 array 类型
    pub is_array: bool,
}

/// WSDL message 只关心其 part 指向的 element 引用
#[derive(Debug, Clone)]
pub struct WsdlMessage {
    pub name: String,
    /// 去掉命名空间前缀后的 element 引用名，如 "AddRequest"
    pub element_ref: String,
}

/// portType/operation 合并了 binding 层的 soapAction 信息，
/// 避免消费者需要同时遍历两个不同层次的 WSDL 节点
#[derive(Debug, Clone)]
pub struct WsdlOperation {
    pub name: String,
    pub input_message: String,
    pub output_message: String,
    pub soap_action: Option<String>,
}

/// 从属性列表中按名称取值，返回 UTF-8 字符串
fn attr_value(e: &BytesStart, name: &[u8]) -> Option<String> {
    e.attributes()
        .filter_map(|a| a.ok())
        .find(|a| a.key.local_name().as_ref() == name)
        .and_then(|a| String::from_utf8(a.value.to_vec()).ok())
}

/// 去掉 "tns:"、"xsd:" 等命名空间前缀，只保留本地名称
fn strip_ns_prefix(s: &str) -> &str {
    s.find(':').map(|i| &s[i + 1..]).unwrap_or(s)
}

/// 解析状态机，每个变体对应 WSDL 文档中的一个层级，
/// 用枚举而非标志位是为了保证同一时刻只处于一个状态，避免条件组合爆炸
#[derive(Debug, Default, PartialEq, Clone)]
enum ParseContext {
    #[default]
    Root,
    /// 处于 <types> 内部
    Types,
    /// 处于某个顶层 <xsd:element> 内部，等待 complexType/sequence
    TypeElement,
    /// 处于 <xsd:sequence> 内部，收集字段列表
    Sequence,
    /// 处于 <message> 内部
    Message,
    /// 处于 <portType> 内部（尚未进入 operation）
    PortType,
    /// 处于 <portType>/<operation> 内部
    PortTypeOperation,
    /// 处于 <binding> 内部（尚未进入 operation）
    Binding,
    /// 处于 <binding>/<operation> 内部
    BindingOperation,
    /// 处于 <service>/<port> 内部
    ServicePort,
}

pub struct WsdlParser;

impl WsdlParser {
    pub fn parse(xml: &str) -> Result<WsdlDefinition> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);

        let mut def = WsdlDefinition {
            service_name: String::new(),
            target_namespace: String::new(),
            endpoint_url: None,
            types: Vec::new(),
            messages: Vec::new(),
            operations: Vec::new(),
        };

        let mut ctx = ParseContext::Root;
        // 当前正在构建的顶层 type（xsd:element）
        let mut current_type: Option<WsdlType> = None;
        // 当前正在构建的 message
        let mut current_message: Option<WsdlMessage> = None;
        // 当前正在构建的 portType operation
        let mut current_op: Option<WsdlOperation> = None;
        // binding 层收集 soapAction，key = operation name
        let mut soap_actions: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        // 当前 binding operation 的名称
        let mut binding_op_name: Option<String> = None;

        let mut buf = Vec::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e) | Event::Empty(ref e)) => {
                    let local = e.local_name();
                    let tag = std::str::from_utf8(local.as_ref())
                        .context("tag name not utf8")?;

                    match (&ctx, tag) {
                        // ── 根节点 ─────────────────────────────────────────────
                        (_, "definitions") => {
                            if let Some(v) = attr_value(e, b"name") {
                                def.service_name = v;
                            }
                            if let Some(v) = attr_value(e, b"targetNamespace") {
                                def.target_namespace = v;
                            }
                        }

                        // ── types 层 ───────────────────────────────────────────
                        (ParseContext::Root, "types") => {
                            ctx = ParseContext::Types;
                        }
                        // xsd:element 在 Types 层出现时是顶层类型定义
                        (ParseContext::Types, "element") => {
                            if let Some(name) = attr_value(e, b"name") {
                                current_type = Some(WsdlType {
                                    name,
                                    elements: Vec::new(),
                                });
                                ctx = ParseContext::TypeElement;
                            }
                        }
                        (ParseContext::TypeElement, "sequence") => {
                            ctx = ParseContext::Sequence;
                        }
                        // xsd:element 在 Sequence 层出现时是字段定义
                        (ParseContext::Sequence, "element") => {
                            if let (Some(name), Some(type_name)) = (
                                attr_value(e, b"name"),
                                attr_value(e, b"type"),
                            ) {
                                let is_array = attr_value(e, b"maxOccurs")
                                    .as_deref()
                                    == Some("unbounded");
                                let type_name =
                                    strip_ns_prefix(&type_name).to_string();
                                if let Some(ref mut t) = current_type {
                                    t.elements.push(WsdlElement {
                                        name,
                                        type_name,
                                        is_array,
                                    });
                                }
                            }
                        }

                        // ── message 层 ─────────────────────────────────────────
                        (ParseContext::Root, "message") => {
                            if let Some(name) = attr_value(e, b"name") {
                                current_message = Some(WsdlMessage {
                                    name,
                                    element_ref: String::new(),
                                });
                                ctx = ParseContext::Message;
                            }
                        }
                        // <part element="tns:AddRequest"/> 自闭合标签
                        (ParseContext::Message, "part") => {
                            if let Some(elem) = attr_value(e, b"element") {
                                if let Some(ref mut msg) = current_message {
                                    msg.element_ref =
                                        strip_ns_prefix(&elem).to_string();
                                }
                            }
                        }

                        // ── portType 层 ────────────────────────────────────────
                        (ParseContext::Root, "portType") => {
                            ctx = ParseContext::PortType;
                        }
                        (ParseContext::PortType, "operation") => {
                            if let Some(name) = attr_value(e, b"name") {
                                current_op = Some(WsdlOperation {
                                    name,
                                    input_message: String::new(),
                                    output_message: String::new(),
                                    soap_action: None,
                                });
                                ctx = ParseContext::PortTypeOperation;
                            }
                        }
                        (ParseContext::PortTypeOperation, "input") => {
                            if let Some(msg) = attr_value(e, b"message") {
                                if let Some(ref mut op) = current_op {
                                    op.input_message =
                                        strip_ns_prefix(&msg).to_string();
                                }
                            }
                        }
                        (ParseContext::PortTypeOperation, "output") => {
                            if let Some(msg) = attr_value(e, b"message") {
                                if let Some(ref mut op) = current_op {
                                    op.output_message =
                                        strip_ns_prefix(&msg).to_string();
                                }
                            }
                        }

                        // ── binding 层 ─────────────────────────────────────────
                        // binding 标签本身的 type 属性不需要，只进入状态
                        (ParseContext::Root, "binding") => {
                            ctx = ParseContext::Binding;
                        }
                        (ParseContext::Binding, "operation") => {
                            if let Some(name) = attr_value(e, b"name") {
                                binding_op_name = Some(name);
                                ctx = ParseContext::BindingOperation;
                            }
                        }
                        // <soap:operation soapAction="..."/> 在 BindingOperation 内
                        (ParseContext::BindingOperation, "operation") => {
                            // 这是 <soap:operation>，本地名称同为 "operation"，
                            // 靠父状态 BindingOperation 区分
                            if let Some(action) = attr_value(e, b"soapAction") {
                                if let Some(ref name) = binding_op_name {
                                    soap_actions.insert(name.clone(), action);
                                }
                            }
                        }

                        // ── service/port 层 ────────────────────────────────────
                        (ParseContext::Root, "service") => {}
                        (_, "port") => {
                            ctx = ParseContext::ServicePort;
                        }
                        // <soap:address location="..."/> 自闭合
                        (ParseContext::ServicePort, "address") => {
                            if let Some(loc) = attr_value(e, b"location") {
                                def.endpoint_url = Some(loc);
                            }
                        }

                        _ => {}
                    }
                }

                Ok(Event::End(ref e)) => {
                    let local = e.local_name();
                    let tag = std::str::from_utf8(local.as_ref())
                        .context("tag name not utf8")?;

                    match (&ctx, tag) {
                        (ParseContext::Types, "types") => {
                            ctx = ParseContext::Root;
                        }
                        // 顶层 xsd:element 结束，把构建好的 type 推入列表
                        (ParseContext::TypeElement, "element") => {
                            if let Some(t) = current_type.take() {
                                def.types.push(t);
                            }
                            ctx = ParseContext::Types;
                        }
                        (ParseContext::Sequence, "sequence") => {
                            ctx = ParseContext::TypeElement;
                        }
                        (ParseContext::Message, "message") => {
                            if let Some(msg) = current_message.take() {
                                def.messages.push(msg);
                            }
                            ctx = ParseContext::Root;
                        }
                        (ParseContext::PortTypeOperation, "operation") => {
                            if let Some(op) = current_op.take() {
                                def.operations.push(op);
                            }
                            ctx = ParseContext::PortType;
                        }
                        (ParseContext::PortType, "portType") => {
                            ctx = ParseContext::Root;
                        }
                        (ParseContext::BindingOperation, "operation") => {
                            binding_op_name = None;
                            ctx = ParseContext::Binding;
                        }
                        (ParseContext::Binding, "binding") => {
                            ctx = ParseContext::Root;
                        }
                        (ParseContext::ServicePort, "port") => {
                            ctx = ParseContext::Root;
                        }
                        _ => {}
                    }
                }

                Ok(Event::Eof) => break,
                Err(e) => return Err(anyhow::anyhow!("XML parse error: {e}")),
                _ => {}
            }
            buf.clear();
        }

        // 将 binding 层收集的 soapAction 回填到 portType 解析出的 operations
        for op in &mut def.operations {
            if let Some(action) = soap_actions.get(&op.name) {
                op.soap_action = Some(action.clone());
            }
        }

        Ok(def)
    }
}

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
        assert_eq!(
            wsdl.endpoint_url.as_deref(),
            Some("http://example.com/calculator")
        );
    }

    #[test]
    fn parses_types() {
        let wsdl = WsdlParser::parse(sample_wsdl()).unwrap();
        assert!(wsdl.types.len() >= 2);
    }

    #[test]
    fn parses_messages() {
        let wsdl = WsdlParser::parse(sample_wsdl()).unwrap();
        assert!(wsdl.messages.len() >= 4);
    }
}
