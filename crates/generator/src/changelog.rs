use serde::Serialize;
use serde_json::Value;

/// 变更日志生成器：递归比较两个 JSON Schema 并输出结构化的变更列表。
/// 主要用于 Contract 版本间的 Breaking Change 检测。
pub struct ChangelogGenerator;

/// 变更类型枚举
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeType {
    Added,
    Removed,
    Modified,
}

/// 一条变更记录，包含字段路径、变更类型、新旧值以及是否为破坏性变更
#[derive(Debug, Clone, Serialize)]
pub struct ChangelogEntry {
    pub field_path: String,
    pub change_type: ChangeType,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    /// 破坏性变更：字段删除、类型变更、新增 required 约束。
    /// 非破坏性变更：字段新增、描述修改、默认值变更。
    pub is_breaking: bool,
}

impl ChangelogGenerator {
    /// 比较两个 JSON Schema（或任意 JSON 值），返回变更列表。
    /// Breaking changes 规则：
    /// - 字段删除 -> breaking
    /// - type 字段值变更 -> breaking
    /// - required 数组新增元素 -> breaking
    /// - 字段新增 -> non-breaking
    /// - description / default 等元数据变更 -> non-breaking
    pub fn diff(old_contract: &Value, new_contract: &Value) -> Vec<ChangelogEntry> {
        let mut entries = Vec::new();
        Self::diff_recursive(old_contract, new_contract, String::new(), &mut entries);
        entries
    }

    fn diff_recursive(
        old: &Value,
        new: &Value,
        path: String,
        entries: &mut Vec<ChangelogEntry>,
    ) {
        match (old, new) {
            (Value::Object(old_map), Value::Object(new_map)) => {
                // 检测删除的字段
                for (key, old_val) in old_map {
                    let field_path = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{}.{}", path, key)
                    };

                    if let Some(new_val) = new_map.get(key) {
                        // 字段仍存在，递归比较
                        Self::diff_recursive(old_val, new_val, field_path, entries);
                    } else {
                        // 字段被删除 -> 总是 breaking
                        entries.push(ChangelogEntry {
                            field_path,
                            change_type: ChangeType::Removed,
                            old_value: Some(Self::value_summary(old_val)),
                            new_value: None,
                            is_breaking: true,
                        });
                    }
                }

                // 检测新增的字段
                for (key, new_val) in new_map {
                    if !old_map.contains_key(key) {
                        let field_path = if path.is_empty() {
                            key.clone()
                        } else {
                            format!("{}.{}", path, key)
                        };

                        // 新增字段默认 non-breaking，但若出现在 required 数组中则需额外判断
                        entries.push(ChangelogEntry {
                            field_path,
                            change_type: ChangeType::Added,
                            old_value: None,
                            new_value: Some(Self::value_summary(new_val)),
                            is_breaking: false,
                        });
                    }
                }

                // 特殊处理 required 数组：新增 required 项是 breaking change
                if let (Some(Value::Array(old_req)), Some(Value::Array(new_req))) =
                    (old_map.get("required"), new_map.get("required"))
                {
                    for item in new_req {
                        if !old_req.contains(item) {
                            let field_path = if path.is_empty() {
                                format!("required[{}]", item)
                            } else {
                                format!("{}.required[{}]", path, item)
                            };
                            entries.push(ChangelogEntry {
                                field_path,
                                change_type: ChangeType::Added,
                                old_value: None,
                                new_value: Some(item.to_string()),
                                is_breaking: true,
                            });
                        }
                    }
                }
            }

            (Value::Array(old_arr), Value::Array(new_arr)) => {
                // 数组比较：按索引逐项比较，多出的项视为新增，少的视为删除
                let max_len = old_arr.len().max(new_arr.len());
                for i in 0..max_len {
                    let item_path = format!("{}[{}]", path, i);
                    match (old_arr.get(i), new_arr.get(i)) {
                        (Some(o), Some(n)) => {
                            Self::diff_recursive(o, n, item_path, entries);
                        }
                        (Some(o), None) => {
                            entries.push(ChangelogEntry {
                                field_path: item_path,
                                change_type: ChangeType::Removed,
                                old_value: Some(Self::value_summary(o)),
                                new_value: None,
                                is_breaking: true,
                            });
                        }
                        (None, Some(n)) => {
                            entries.push(ChangelogEntry {
                                field_path: item_path,
                                change_type: ChangeType::Added,
                                old_value: None,
                                new_value: Some(Self::value_summary(n)),
                                is_breaking: false,
                            });
                        }
                        (None, None) => unreachable!(),
                    }
                }
            }

            // 叶子节点值变更
            _ => {
                if old != new {
                    // "type" 字段变更是 breaking（类型不兼容），其他叶子节点变更是 non-breaking
                    let is_breaking = path.ends_with(".type") || path == "type";
                    entries.push(ChangelogEntry {
                        field_path: path,
                        change_type: ChangeType::Modified,
                        old_value: Some(Self::value_summary(old)),
                        new_value: Some(Self::value_summary(new)),
                        is_breaking,
                    });
                }
            }
        }
    }

    /// 生成值的简短文本摘要，避免在变更日志中展示过长的 JSON
    fn value_summary(value: &Value) -> String {
        match value {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Null => "null".to_string(),
            // 对象和数组只展示类型标识，完整内容由调用方按需获取
            Value::Object(_) => "{...}".to_string(),
            Value::Array(a) => format!("[...] ({} items)", a.len()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detects_added_field() {
        let old = json!({"properties": {"name": {"type": "string"}}});
        let new = json!({"properties": {"name": {"type": "string"}, "age": {"type": "integer"}}});

        let entries = ChangelogGenerator::diff(&old, &new);
        let added = entries.iter().find(|e| e.field_path == "properties.age").unwrap();
        assert_eq!(added.change_type, ChangeType::Added);
        assert!(!added.is_breaking, "新增字段不应是破坏性变更");
    }

    #[test]
    fn detects_removed_field() {
        let old = json!({"properties": {"name": {"type": "string"}, "age": {"type": "integer"}}});
        let new = json!({"properties": {"name": {"type": "string"}}});

        let entries = ChangelogGenerator::diff(&old, &new);
        let removed = entries.iter().find(|e| e.field_path == "properties.age").unwrap();
        assert_eq!(removed.change_type, ChangeType::Removed);
        assert!(removed.is_breaking, "删除字段应是破坏性变更");
    }

    #[test]
    fn detects_type_change_as_breaking() {
        let old = json!({"properties": {"name": {"type": "string"}}});
        let new = json!({"properties": {"name": {"type": "integer"}}});

        let entries = ChangelogGenerator::diff(&old, &new);
        let modified = entries.iter().find(|e| e.field_path == "properties.name.type").unwrap();
        assert_eq!(modified.change_type, ChangeType::Modified);
        assert!(modified.is_breaking, "类型变更应是破坏性变更");
        assert_eq!(modified.old_value.as_deref(), Some("string"));
        assert_eq!(modified.new_value.as_deref(), Some("integer"));
    }

    #[test]
    fn detects_new_required_as_breaking() {
        let old = json!({"required": ["name"]});
        let new = json!({"required": ["name", "age"]});

        let entries = ChangelogGenerator::diff(&old, &new);
        let added_req = entries.iter().find(|e| e.field_path.contains("required") && e.field_path.contains("age")).unwrap();
        assert!(added_req.is_breaking, "新增 required 约束应是破坏性变更");
    }

    #[test]
    fn description_change_is_not_breaking() {
        let old = json!({"description": "Old description"});
        let new = json!({"description": "New description"});

        let entries = ChangelogGenerator::diff(&old, &new);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].change_type, ChangeType::Modified);
        assert!(!entries[0].is_breaking, "描述变更不应是破坏性变更");
    }

    #[test]
    fn no_changes_returns_empty() {
        let schema = json!({"type": "object", "properties": {"name": {"type": "string"}}});
        let entries = ChangelogGenerator::diff(&schema, &schema);
        assert!(entries.is_empty());
    }

    #[test]
    fn complex_nested_diff() {
        let old = json!({
            "type": "object",
            "required": ["id"],
            "properties": {
                "id": {"type": "integer"},
                "name": {"type": "string"},
                "address": {
                    "type": "object",
                    "properties": {
                        "street": {"type": "string"},
                        "city": {"type": "string"}
                    }
                }
            }
        });
        let new = json!({
            "type": "object",
            "required": ["id", "name"],
            "properties": {
                "id": {"type": "integer"},
                "name": {"type": "integer"},
                "email": {"type": "string"}
            }
        });

        let entries = ChangelogGenerator::diff(&old, &new);

        // name 类型从 string 变为 integer -> breaking
        assert!(entries.iter().any(|e| e.field_path == "properties.name.type" && e.is_breaking));
        // address 被删除 -> breaking
        assert!(entries.iter().any(|e| e.field_path == "properties.address" && e.change_type == ChangeType::Removed));
        // email 新增 -> non-breaking
        assert!(entries.iter().any(|e| e.field_path == "properties.email" && !e.is_breaking));
        // required 新增 "name" -> breaking
        assert!(entries.iter().any(|e| e.field_path.contains("required") && e.field_path.contains("name") && e.is_breaking));
    }
}
