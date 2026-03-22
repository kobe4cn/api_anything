use rand::Rng;
use serde_json::{json, Value};
use uuid::Uuid;

pub struct MockLayer;

impl MockLayer {
    /// 从响应 schema 生成 mock 数据，config 中的 fixed_response 优先级最高，
    /// 允许测试方绕过 schema 推断直接返回预设数据
    pub fn generate(schema: &Value, config: &Value) -> Value {
        if let Some(fixed) = config.get("fixed_response") {
            return fixed.clone();
        }
        Self::generate_from_schema(schema)
    }

    pub fn generate_from_schema(schema: &Value) -> Value {
        match schema.get("type").and_then(|t| t.as_str()) {
            Some("object") => {
                let mut obj = serde_json::Map::new();
                if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
                    for (key, prop_schema) in props {
                        obj.insert(key.clone(), Self::generate_field(key, prop_schema));
                    }
                }
                Value::Object(obj)
            }
            Some("array") => {
                // 把默认 schema 绑定为局部变量，延长生命周期到整个分支结束，
                // 避免 unwrap_or 中 json! 临时值被提前释放而导致悬空引用
                let default_item = json!({"type": "string"});
                let item_schema = schema.get("items").unwrap_or(&default_item);
                // 生成 1-3 个元素，数量随机以模拟真实列表的多样性
                let count = rand::thread_rng().gen_range(1..=3);
                let items: Vec<Value> = (0..count)
                    .map(|_| Self::generate_from_schema(item_schema))
                    .collect();
                Value::Array(items)
            }
            Some("string") => {
                // enum 约束下只能返回合法值，否则消费方的枚举校验会失败
                if let Some(values) = schema.get("enum").and_then(|e| e.as_array()) {
                    let idx = rand::thread_rng().gen_range(0..values.len());
                    return values[idx].clone();
                }
                json!("sample-string")
            }
            Some("integer") => json!(rand::thread_rng().gen_range(1..=100)),
            Some("number") => json!(rand::thread_rng().gen_range(1.0..=100.0_f64)),
            Some("boolean") => json!(rand::thread_rng().gen_bool(0.5)),
            _ => json!(null),
        }
    }

    /// 根据字段名语义推断合适的 mock 值，优先于 schema 类型推断，
    /// 使生成数据在业务层面更具可读性，方便开发者直观判断字段含义
    fn generate_field(field_name: &str, schema: &Value) -> Value {
        let lower = field_name.to_lowercase();

        if lower.contains("email") {
            return json!("user@example.com");
        }
        if lower.contains("phone") {
            return json!("+86-13800001234");
        }
        if lower == "name" || lower.contains("username") {
            return json!("John Doe");
        }
        // id 字段用 UUID 而非递增整数，避免测试方对固定 id 产生隐式依赖
        if lower == "id" || lower.ends_with("_id") {
            return json!(Uuid::new_v4().to_string());
        }
        if lower.contains("amount") || lower.contains("price") {
            return json!(99.50);
        }
        if lower.contains("date") || lower.contains("created_at") || lower.contains("updated_at") {
            return json!("2024-01-15T10:30:00Z");
        }
        if lower == "status" {
            return json!("active");
        }
        if lower.contains("count") || lower.contains("total") {
            return json!(42);
        }
        if lower.contains("url") || lower.contains("link") {
            return json!("https://example.com");
        }
        if lower.contains("description") {
            return json!("Sample description text");
        }

        // 字段名无语义线索时，退回到 schema 类型推断
        Self::generate_from_schema(schema)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn generates_object_from_schema() {
        let schema = json!({
            "type": "object",
            "properties": {
                "result": {"type": "integer"},
                "message": {"type": "string"}
            }
        });
        let result = MockLayer::generate(&schema, &json!({}));
        assert!(result["result"].is_number());
        assert!(result["message"].is_string());
    }

    #[test]
    fn smart_mock_email_field() {
        let schema = json!({
            "type": "object",
            "properties": { "email": {"type": "string"} }
        });
        let result = MockLayer::generate(&schema, &json!({}));
        assert!(result["email"].as_str().unwrap().contains("@"));
    }

    #[test]
    fn respects_enum_values() {
        let schema = json!({
            "type": "object",
            "properties": {
                "status": {"type": "string", "enum": ["active", "inactive", "pending"]}
            }
        });
        let result = MockLayer::generate(&schema, &json!({}));
        let status = result["status"].as_str().unwrap();
        assert!(["active", "inactive", "pending"].contains(&status));
    }

    #[test]
    fn returns_fixed_response_when_configured() {
        let schema = json!({"type": "object"});
        let config = json!({"fixed_response": {"custom": "value"}});
        let result = MockLayer::generate(&schema, &config);
        assert_eq!(result["custom"], "value");
    }
}
