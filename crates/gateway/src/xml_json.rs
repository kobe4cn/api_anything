use anyhow::Result;
use quick_xml::events::Event;
use quick_xml::Reader;
use serde_json::{Map, Value};

/// 将 JSON 值递归转换为 XML 元素文本；
/// 用于构造 SOAP 请求体，避免引入重型 XML 序列化框架
pub struct SoapXmlBuilder;

impl SoapXmlBuilder {
    /// 从 JSON body 构建完整的 SOAP Envelope。
    /// 顶层字段直接映射为 `<ns:OperationName>` 的子元素，
    /// 嵌套对象和数组则递归处理
    pub fn build_envelope(
        soap_action: &str,
        operation_name: &str,
        namespace: &str,
        body: &Value,
    ) -> String {
        let mut xml = String::new();
        xml.push_str(r#"<?xml version="1.0" encoding="utf-8"?>"#);
        xml.push_str(&format!(
            r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/" xmlns:ns="{namespace}">"#,
        ));
        xml.push_str("<soap:Body>");
        xml.push_str(&format!("<ns:{operation_name}>"));

        // SOAPAction 只写入 HTTP Header，此处仅让编译器不警告未使用
        let _ = soap_action;

        Self::write_value(&mut xml, body);

        xml.push_str(&format!("</ns:{operation_name}>"));
        xml.push_str("</soap:Body>");
        xml.push_str("</soap:Envelope>");
        xml
    }

    /// 将单个 JSON 值序列化为 XML 片段。
    /// - 对象：每个字段独立包裹在同名元素内
    /// - 数组：重复相同父标签（SOAP 惯例），由调用方传入 tag 名
    /// - 标量：直接作为文本节点写入
    fn write_value(out: &mut String, value: &Value) {
        match value {
            Value::Object(map) => {
                for (key, val) in map {
                    out.push_str(&format!("<{key}>"));
                    Self::write_value(out, val);
                    out.push_str(&format!("</{key}>"));
                }
            }
            Value::Array(items) => {
                // SOAP 没有原生数组概念，重复同名元素是最常见的编码惯例
                for item in items {
                    Self::write_value(out, item);
                }
            }
            Value::String(s) => {
                out.push_str(&Self::escape_xml(s));
            }
            Value::Number(n) => {
                out.push_str(&n.to_string());
            }
            Value::Bool(b) => {
                out.push_str(if *b { "true" } else { "false" });
            }
            Value::Null => {}
        }
    }

    /// XML 文本节点的特殊字符必须转义，防止注入破坏 Envelope 结构
    fn escape_xml(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }
}

/// 从 SOAP XML 响应中提取 JSON；
/// 只解析 `<soap:Body>` 内部内容，跳过根响应元素（如 `<AddResponse>`），
/// 直接将子元素映射为 JSON key-value
pub struct SoapXmlParser;

impl SoapXmlParser {
    /// 解析 SOAP XML 响应，返回 Body 内容对应的 JSON 对象。
    /// 空 Body（`<soap:Body/>`）返回空对象，符合"无结果也是合法响应"的 SOAP 语义
    pub fn parse_response(xml: &str) -> Result<Value> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);

        // 对象栈：每层代表一个嵌套 XML 元素对应的 JSON 对象，
        // 最终展开为树形结构，避免了路径字符串拼接的复杂性
        //
        // 状态：
        //   in_body              = 已进入 <soap:Body>
        //   root_response_tag    = 根响应元素名（如 "AddResponse"），用于识别它的关闭标签
        //   obj_stack            = 当前嵌套路径上的各层 JSON 对象
        let mut in_body = false;
        let mut root_response_tag: Option<String> = None;
        // (tag_name, partially_built_object) 的栈，栈底为哨兵收集器 "__root__"
        let mut obj_stack: Vec<(String, Map<String, Value>)> = Vec::new();
        let mut text_buf = String::new();

        loop {
            match reader.read_event() {
                Ok(Event::Start(ref e)) => {
                    let raw_name = std::str::from_utf8(e.name().as_ref())?.to_string();
                    let local = strip_prefix(&raw_name).to_string();

                    if !in_body {
                        if local == "Body" {
                            in_body = true;
                        }
                        continue;
                    }

                    if root_response_tag.is_none() {
                        // 第一层是根响应元素（如 AddResponse），记住其名字以便识别闭合；
                        // 在栈底放哨兵收集器，后续子元素都归入其中
                        root_response_tag = Some(local);
                        obj_stack.push(("__root__".to_string(), Map::new()));
                        continue;
                    }

                    // 普通子元素：压栈一个空对象，待关闭时填充
                    obj_stack.push((local, Map::new()));
                    text_buf.clear();
                }
                Ok(Event::Empty(ref e)) => {
                    let raw_name = std::str::from_utf8(e.name().as_ref())?.to_string();
                    let local = strip_prefix(&raw_name);
                    if local == "Body" {
                        // `<soap:Body/>` — 空 Body，直接结束
                        return Ok(Value::Object(Map::new()));
                    }
                }
                Ok(Event::Text(ref e)) => {
                    if in_body && root_response_tag.is_some() {
                        text_buf.push_str(&e.unescape()?.to_string());
                    }
                }
                Ok(Event::End(ref e)) => {
                    let raw_name = std::str::from_utf8(e.name().as_ref())?.to_string();
                    let local = strip_prefix(&raw_name).to_string();

                    if local == "Body" {
                        // Body 关闭，无论当前状态都结束解析
                        break;
                    }

                    if !in_body {
                        continue;
                    }

                    // 根响应元素关闭时直接 break，不 pop 栈（哨兵收集器留在栈中供后续提取）
                    if root_response_tag.as_deref() == Some(&local) {
                        break;
                    }

                    // 普通子元素关闭：弹出并合并到父层
                    if let Some((tag, child_obj)) = obj_stack.pop() {
                        // 有子字段 → JSON 对象；无子字段 → 文本节点
                        let value = if child_obj.is_empty() {
                            Value::String(text_buf.clone())
                        } else {
                            Value::Object(child_obj)
                        };
                        text_buf.clear();

                        if let Some((_parent_tag, parent_obj)) = obj_stack.last_mut() {
                            // 合并到父对象（包括哨兵收集器 __root__）
                            parent_obj.insert(tag, value);
                        }
                        // 若栈已空说明 XML 格式异常，静默忽略以保持鲁棒性
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(anyhow::anyhow!("XML parse error: {}", e)),
                _ => {}
            }
        }

        // 哨兵收集器 "__root__" 留在栈底，其内容即为最终结果；
        // 若栈为空（空 Body 或格式异常）则返回空对象
        let result = obj_stack
            .into_iter()
            .find(|(tag, _)| tag == "__root__")
            .map(|(_, obj)| obj)
            .unwrap_or_default();
        Ok(Value::Object(result))
    }
}

/// 剥离 XML 命名空间前缀（`soap:Body` → `Body`）
fn strip_prefix(name: &str) -> &str {
    name.find(':').map(|i| &name[i + 1..]).unwrap_or(name)
}

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
        assert!(xml.contains("Add")); // operation name in body
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
