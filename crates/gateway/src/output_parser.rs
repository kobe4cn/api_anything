use api_anything_common::error::AppError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 三种输出解析模式；由路由配置决定，不同命令的输出格式差异大，
/// 因此需要在注册路由时显式指定解析策略而非猜测
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OutputFormat {
    /// 命令输出本身就是 JSON，直接反序列化
    Json,
    /// 用正则模式从非结构化文本中提取字段；
    /// 每个 pattern 的第一个捕获组作为字段值
    Regex { patterns: Vec<RegexPattern> },
    /// 不做解析，将 stdout 原文返回给调用方
    RawText,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegexPattern {
    pub field_name: String,
    /// 正则表达式字符串，第一个捕获组 `(...)` 即目标值
    pub pattern: String,
}

pub struct OutputParser;

impl OutputParser {
    pub fn parse(output: &str, format: &OutputFormat) -> Result<Value, AppError> {
        match format {
            OutputFormat::Json => {
                serde_json::from_str(output)
                    .map_err(|e| AppError::Internal(format!("JSON parse error: {e}")))
            }
            OutputFormat::Regex { patterns } => {
                let mut result = serde_json::Map::new();
                for p in patterns {
                    let re = regex::Regex::new(&p.pattern)
                        .map_err(|e| AppError::Internal(format!("Invalid regex: {e}")))?;
                    if let Some(caps) = re.captures(output) {
                        if let Some(val) = caps.get(1) {
                            result.insert(
                                p.field_name.clone(),
                                Value::String(val.as_str().to_string()),
                            );
                        }
                    }
                }
                Ok(Value::Object(result))
            }
            OutputFormat::RawText => Ok(json!({ "stdout": output })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_output() {
        let output = r#"{"name": "test", "count": 42}"#;
        let result = OutputParser::parse(output, &OutputFormat::Json).unwrap();
        assert_eq!(result["name"], "test");
        assert_eq!(result["count"], 42);
    }

    #[test]
    fn parses_with_regex_patterns() {
        let output = "Report ID: R-001\nTotal rows: 42\nStatus: complete";
        let format = OutputFormat::Regex {
            patterns: vec![
                RegexPattern {
                    field_name: "report_id".into(),
                    pattern: r"Report ID: (\S+)".into(),
                },
                RegexPattern {
                    field_name: "total_rows".into(),
                    pattern: r"Total rows: (\d+)".into(),
                },
                RegexPattern {
                    field_name: "status".into(),
                    pattern: r"Status: (\w+)".into(),
                },
            ],
        };
        let result = OutputParser::parse(output, &format).unwrap();
        assert_eq!(result["report_id"], "R-001");
        assert_eq!(result["total_rows"], "42");
        assert_eq!(result["status"], "complete");
    }

    #[test]
    fn returns_raw_text() {
        let output = "some raw output\nwith multiple lines";
        let result = OutputParser::parse(output, &OutputFormat::RawText).unwrap();
        assert_eq!(result["stdout"], output);
    }

    #[test]
    fn handles_invalid_json_gracefully() {
        let result = OutputParser::parse("not json", &OutputFormat::Json);
        assert!(result.is_err());
    }
}
