use anyhow::{anyhow, Result};
use regex::Regex;

/// CLI 工具的完整定义，从主帮助文本解析而来
#[derive(Debug, Clone)]
pub struct CliDefinition {
    pub program_name: String,
    pub description: String,
    pub subcommands: Vec<CliSubcommand>,
    pub global_options: Vec<CliOption>,
}

/// 子命令定义，初始由主帮助文本提供名称和简述，
/// 后续可通过 parse_subcommand 丰富其 options 列表
#[derive(Debug, Clone)]
pub struct CliSubcommand {
    pub name: String,
    pub description: String,
    pub options: Vec<CliOption>,
}

/// 单个 CLI 选项，支持短标志、长标志、值占位符及约束信息
#[derive(Debug, Clone)]
pub struct CliOption {
    pub short: Option<String>,
    pub long: Option<String>,
    pub value_name: Option<String>,
    pub description: String,
    /// 若选项在 USAGE 行中不带方括号出现，则视为必填
    pub required: bool,
    pub default_value: Option<String>,
    pub possible_values: Vec<String>,
}

/// 解析 CLIクラップ（clap）风格的帮助文本，提取结构化定义
pub struct CliHelpParser;

impl CliHelpParser {
    /// 解析主帮助文本（`program --help` 输出），提取程序名、描述、子命令和全局选项。
    /// 程序名取第一行第一个空白分隔的词，描述取第二行（若非空）。
    pub fn parse_main(help_text: &str) -> Result<CliDefinition> {
        let lines: Vec<&str> = help_text.lines().collect();

        if lines.is_empty() {
            return Err(anyhow!("help text is empty"));
        }

        // 第一行格式通常为 "program-name version"，取首词作为程序名
        let program_name = lines[0]
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string();

        // 第二行（跳过空行）作为简短描述
        let description = lines
            .iter()
            .skip(1)
            .find(|l| !l.trim().is_empty())
            .map(|l| l.trim().to_string())
            .unwrap_or_default();

        // 提取 USAGE: 后的实际用法行，用于判断全局选项是否必填
        let usage_line = {
            let mut found = String::new();
            let mut after_usage = false;
            for line in &lines {
                if line.trim().eq_ignore_ascii_case("usage:") {
                    after_usage = true;
                    continue;
                }
                if after_usage && !line.trim().is_empty() {
                    found = line.to_string();
                    break;
                }
            }
            found
        };

        let subcommands = Self::parse_subcommands_section(&lines);
        let global_options = Self::parse_options_section(&lines, &usage_line);

        Ok(CliDefinition {
            program_name,
            description,
            subcommands,
            global_options,
        })
    }

    /// 解析子命令帮助文本（`program subcommand --help` 输出），提取该子命令的选项列表。
    /// 子命令名取第一行最后一个词（`program subcommand` 格式）。
    pub fn parse_subcommand(help_text: &str) -> Result<CliSubcommand> {
        let lines: Vec<&str> = help_text.lines().collect();

        if lines.is_empty() {
            return Err(anyhow!("subcommand help text is empty"));
        }

        // 第一行格式为 "program-name subcommand-name"，取最后一词
        let name = lines[0]
            .split_whitespace()
            .last()
            .unwrap_or("")
            .to_string();

        let description = lines
            .iter()
            .skip(1)
            .find(|l| !l.trim().is_empty())
            .map(|l| l.trim().to_string())
            .unwrap_or_default();

        // 找到 USAGE: 标题，然后取紧跟的那行作为真正的用法行，
        // 避免把第一行（"program-name subcommand-name"）误判为 USAGE 行
        let usage_line = {
            let mut found = String::new();
            let mut after_usage = false;
            for line in &lines {
                if line.trim().eq_ignore_ascii_case("usage:") {
                    after_usage = true;
                    continue;
                }
                if after_usage {
                    if !line.trim().is_empty() {
                        found = line.to_string();
                        break;
                    }
                }
            }
            found
        };

        let options = Self::parse_options_section(&lines, &usage_line);

        Ok(CliSubcommand {
            name,
            description,
            options,
        })
    }

    /// 从行列表中找到 SUBCOMMANDS: 段落，解析每一行的子命令名和简述
    fn parse_subcommands_section(lines: &[&str]) -> Vec<CliSubcommand> {
        let mut subcommands = Vec::new();
        let mut in_section = false;

        for line in lines {
            let trimmed = line.trim();

            if trimmed.eq_ignore_ascii_case("subcommands:") {
                in_section = true;
                continue;
            }

            // 遇到下一个段落标题（全大写后跟冒号）时退出
            if in_section {
                if trimmed.is_empty() {
                    continue;
                }
                // 段落标题特征：行首非空格，且以冒号结尾（如 OPTIONS:）
                if !line.starts_with(' ') && trimmed.ends_with(':') {
                    break;
                }
                // 子命令行格式：前导空格 + 名称 + 空格 + 描述
                if let Some((name, desc)) = Self::split_name_desc(trimmed) {
                    subcommands.push(CliSubcommand {
                        name,
                        description: desc,
                        options: Vec::new(),
                    });
                }
            }
        }

        subcommands
    }

    /// 从行列表中找到 OPTIONS: 段落，解析每一行的选项定义。
    /// 跳过 -h/--help 和 -V/--version 等元信息选项，它们对 API 映射无意义。
    fn parse_options_section(lines: &[&str], usage_line: &str) -> Vec<CliOption> {
        let mut options = Vec::new();
        let mut in_section = false;

        // 预编译选项行正则：匹配 `-s, --long <VALUE>   description` 格式
        // 短标志和长标志都可选，value_name 用尖括号包裹
        let opt_re = Regex::new(
            r"^\s+(-(\w)),?\s+(--[\w-]+)(?:\s+<(\w+)>)?\s{2,}(.+)$"
        ).unwrap();

        // 仅有长标志（无短标志）的选项：`--long <VALUE>   description`
        let long_only_re = Regex::new(
            r"^\s+(--[\w-]+)(?:\s+<(\w+)>)?\s{2,}(.+)$"
        ).unwrap();

        for line in lines {
            let trimmed = line.trim();

            if trimmed.eq_ignore_ascii_case("options:") {
                in_section = true;
                continue;
            }

            if in_section {
                if trimmed.is_empty() {
                    continue;
                }
                if !line.starts_with(' ') && trimmed.ends_with(':') {
                    break;
                }

                // 尝试匹配带短标志的完整格式
                if let Some(caps) = opt_re.captures(line) {
                    let short_char = caps.get(2).map(|m| m.as_str().to_string());
                    let long_flag = caps.get(3).map(|m| m.as_str().to_string());
                    let value_name = caps.get(4).map(|m| m.as_str().to_string());
                    let raw_desc = caps.get(5).map(|m| m.as_str()).unwrap_or("").trim();

                    // 跳过 help 和 version 这类工具元选项
                    if Self::is_meta_option(short_char.as_deref(), long_flag.as_deref()) {
                        continue;
                    }

                    let required = Self::detect_required(&long_flag, usage_line);
                    let default_value = Self::extract_default(raw_desc);
                    let possible_values = Self::extract_possible_values(raw_desc);
                    let description = Self::clean_description(raw_desc);

                    options.push(CliOption {
                        short: short_char.map(|c| format!("-{c}")),
                        long: long_flag,
                        value_name,
                        description,
                        required,
                        default_value,
                        possible_values,
                    });
                } else if let Some(caps) = long_only_re.captures(line) {
                    // 仅长标志格式（无 -x 短标志）
                    let long_flag = caps.get(1).map(|m| m.as_str().to_string());
                    let value_name = caps.get(2).map(|m| m.as_str().to_string());
                    let raw_desc = caps.get(3).map(|m| m.as_str()).unwrap_or("").trim();

                    if Self::is_meta_option(None, long_flag.as_deref()) {
                        continue;
                    }

                    let required = Self::detect_required(&long_flag, usage_line);
                    let default_value = Self::extract_default(raw_desc);
                    let possible_values = Self::extract_possible_values(raw_desc);
                    let description = Self::clean_description(raw_desc);

                    options.push(CliOption {
                        short: None,
                        long: long_flag,
                        value_name,
                        description,
                        required,
                        default_value,
                        possible_values,
                    });
                }
            }
        }

        options
    }

    /// 判断是否为工具元选项（help、version），这类选项不映射为 API 参数
    fn is_meta_option(short: Option<&str>, long: Option<&str>) -> bool {
        matches!(short, Some("h") | Some("V"))
            || matches!(long, Some("--help") | Some("--version"))
    }

    /// 在 USAGE 行中查找该选项是否不带方括号出现，不带方括号即为必填。
    /// 例如 `--type <TYPE>` 是必填，`[--format <FORMAT>]` 是可选。
    fn detect_required(long_flag: &Option<String>, usage_line: &str) -> bool {
        let Some(flag) = long_flag else { return false };
        // 去掉 "--" 前缀取标志名，在 USAGE 中检查是否在方括号外出现
        let flag_name = flag.trim_start_matches('-');
        let usage = usage_line;
        if !usage.contains(flag_name) {
            return false;
        }
        // 若 USAGE 中该标志被 [ ] 包裹，则为可选
        !usage.contains(&format!("[--{}", flag_name))
            && !usage.contains(&format!("[-{}", flag_name))
    }

    /// 从描述字符串中提取 `[default: value]` 中的默认值
    fn extract_default(desc: &str) -> Option<String> {
        let re = Regex::new(r"\[default:\s*([^\]]+)\]").unwrap();
        re.captures(desc)
            .and_then(|caps| caps.get(1))
            .map(|m| m.as_str().trim().to_string())
    }

    /// 从描述字符串中提取 `[possible values: a, b, c]` 中的枚举列表
    fn extract_possible_values(desc: &str) -> Vec<String> {
        let re = Regex::new(r"\[possible values:\s*([^\]]+)\]").unwrap();
        if let Some(caps) = re.captures(desc) {
            if let Some(m) = caps.get(1) {
                return m
                    .as_str()
                    .split(',')
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
                    .collect();
            }
        }
        Vec::new()
    }

    /// 去除描述中的 `[default: ...]` 和 `[possible values: ...]` 注解，
    /// 保留对用户有意义的纯文本描述
    fn clean_description(desc: &str) -> String {
        let re = Regex::new(r"\[(default|possible values):[^\]]*\]").unwrap();
        re.replace_all(desc, "").trim().to_string()
    }

    /// 将 "name    description" 格式的行按首段空白分割为 (name, description)
    fn split_name_desc(line: &str) -> Option<(String, String)> {
        // 至少两个连续空格才算分隔符，单空格可能是名称本身的一部分
        let re = Regex::new(r"^(\S+)\s{2,}(.+)$").unwrap();
        re.captures(line).map(|caps| {
            (
                caps.get(1).map(|m| m.as_str().to_string()).unwrap_or_default(),
                caps.get(2).map(|m| m.as_str().trim().to_string()).unwrap_or_default(),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_program_name() {
        let def = CliHelpParser::parse_main(include_str!("../../tests/fixtures/sample_help.txt")).unwrap();
        assert_eq!(def.program_name, "report-gen");
    }

    #[test]
    fn parses_subcommands() {
        let def = CliHelpParser::parse_main(include_str!("../../tests/fixtures/sample_help.txt")).unwrap();
        assert_eq!(def.subcommands.len(), 3);
        assert!(def.subcommands.iter().any(|s| s.name == "generate"));
        assert!(def.subcommands.iter().any(|s| s.name == "list"));
    }

    #[test]
    fn parses_subcommand_options() {
        let sub = CliHelpParser::parse_subcommand(include_str!("../../tests/fixtures/sample_subcommand_help.txt")).unwrap();
        // 应包含 type、format、start、end、output（共5个），不含 help
        assert!(sub.options.len() >= 4);
        let type_opt = sub.options.iter().find(|o| o.long.as_deref() == Some("--type")).unwrap();
        assert!(type_opt.required);
    }

    #[test]
    fn detects_default_values() {
        let sub = CliHelpParser::parse_subcommand(include_str!("../../tests/fixtures/sample_subcommand_help.txt")).unwrap();
        let fmt = sub.options.iter().find(|o| o.long.as_deref() == Some("--format")).unwrap();
        assert_eq!(fmt.default_value.as_deref(), Some("json"));
    }

    #[test]
    fn detects_possible_values() {
        let sub = CliHelpParser::parse_subcommand(include_str!("../../tests/fixtures/sample_subcommand_help.txt")).unwrap();
        let fmt = sub.options.iter().find(|o| o.long.as_deref() == Some("--format")).unwrap();
        assert_eq!(fmt.possible_values, vec!["json", "csv", "html"]);
    }
}
