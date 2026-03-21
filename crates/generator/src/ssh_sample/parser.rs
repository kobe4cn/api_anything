use anyhow::{anyhow, Result};
use regex::Regex;

/// SSH 交互样本的完整定义，从纯文本样本文件解析而来。
/// 约定用 `#` 开头的行描述连接信息，`##` 开头的行开始命令块。
#[derive(Debug, Clone)]
pub struct SshSampleDefinition {
    pub host: String,
    pub user: String,
    pub description: String,
    pub commands: Vec<SshCommand>,
}

/// 单个 SSH 命令的结构化描述，包含命令模板、文档和示例输出。
/// `parameters` 由命令模板中 `{param}` 占位符提取，供路径参数映射使用。
#[derive(Debug, Clone)]
pub struct SshCommand {
    pub command_template: String,
    pub description: String,
    pub output_format: String,
    pub parameters: Vec<String>,
    pub sample_output: String,
}

pub struct SshSampleParser;

impl SshSampleParser {
    /// 解析 SSH 交互样本文本，提取连接信息和命令列表。
    ///
    /// 文件格式约定：
    /// - `# Host:` / `# User:` / `# Description:` 位于文件头部，顺序不限
    /// - 每个命令块以 `## Command:` 开始，后续跟随 `## Description:`、
    ///   `## Output Format:`、`## Sample Output:`（此后至下一命令块或 EOF 为样例输出内容）
    pub fn parse(text: &str) -> Result<SshSampleDefinition> {
        let param_re = Regex::new(r"\{(\w+)\}").unwrap();

        let mut host = String::new();
        let mut user = String::new();
        let mut description = String::new();
        let mut commands: Vec<SshCommand> = Vec::new();

        // 当前正在构建的命令块上下文，None 表示尚未进入任何命令块
        let mut current_cmd: Option<CurrentCommand> = None;
        // 标记当前行是否属于 Sample Output 段落（多行内容采集）
        let mut in_sample_output = false;

        for line in text.lines() {
            // 遇到新命令块时，先落盘当前未完成的命令，再初始化新块
            if let Some(cmd_template) = line.strip_prefix("## Command:") {
                if let Some(prev) = current_cmd.take() {
                    commands.push(finalize_command(prev, &param_re));
                }
                let template = cmd_template.trim().to_string();
                current_cmd = Some(CurrentCommand {
                    command_template: template,
                    description: String::new(),
                    output_format: String::new(),
                    sample_output_lines: Vec::new(),
                });
                in_sample_output = false;
                continue;
            }

            // 命令块内的子字段解析
            if let Some(ref mut cmd) = current_cmd {
                if let Some(val) = line.strip_prefix("## Description:") {
                    cmd.description = val.trim().to_string();
                    in_sample_output = false;
                } else if let Some(val) = line.strip_prefix("## Output Format:") {
                    cmd.output_format = val.trim().to_string();
                    in_sample_output = false;
                } else if line.trim() == "## Sample Output:" {
                    // 标记后续行归属 Sample Output，直到下一个 ## 字段或 EOF
                    in_sample_output = true;
                } else if in_sample_output {
                    // 空行在 Sample Output 段落中可能代表格式间距，保留
                    cmd.sample_output_lines.push(line.to_string());
                }
                continue;
            }

            // 文件头部：`# Key: Value` 格式的连接元信息
            if let Some(rest) = line.strip_prefix("# ") {
                if let Some(val) = rest.strip_prefix("Host:") {
                    host = val.trim().to_string();
                } else if let Some(val) = rest.strip_prefix("User:") {
                    user = val.trim().to_string();
                } else if let Some(val) = rest.strip_prefix("Description:") {
                    description = val.trim().to_string();
                }
            }
        }

        // 文件结尾时落盘最后一个命令块
        if let Some(prev) = current_cmd.take() {
            commands.push(finalize_command(prev, &param_re));
        }

        if host.is_empty() {
            return Err(anyhow!("SSH sample missing required '# Host:' header"));
        }
        if user.is_empty() {
            return Err(anyhow!("SSH sample missing required '# User:' header"));
        }

        Ok(SshSampleDefinition {
            host,
            user,
            description,
            commands,
        })
    }
}

/// 命令块的临时构建状态，避免将 Vec<String> 直接暴露在最终结构中
struct CurrentCommand {
    command_template: String,
    description: String,
    output_format: String,
    /// 逐行采集 Sample Output 段落内容，最终 join 为单个字符串
    sample_output_lines: Vec<String>,
}

/// 将临时命令状态转换为最终的 SshCommand，同时提取模板中的路径参数名
fn finalize_command(cmd: CurrentCommand, param_re: &Regex) -> SshCommand {
    // 去除 Sample Output 末尾的空白行，避免干扰后续字符串比较
    let trimmed_lines: Vec<&str> = cmd.sample_output_lines
        .iter()
        .map(|l| l.as_str())
        .collect();
    let sample_output = trim_trailing_empty(trimmed_lines).join("\n");

    let parameters: Vec<String> = param_re
        .captures_iter(&cmd.command_template)
        .map(|cap| cap[1].to_string())
        .collect();

    SshCommand {
        command_template: cmd.command_template,
        description: cmd.description,
        output_format: cmd.output_format,
        parameters,
        sample_output,
    }
}

/// 去除末尾的空白行，保留中间的空白行（维持 Sample Output 的视觉格式）
fn trim_trailing_empty(lines: Vec<&str>) -> Vec<&str> {
    let mut end = lines.len();
    while end > 0 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    lines[..end].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_host_and_user() {
        let def = SshSampleParser::parse(
            include_str!("../../tests/fixtures/ssh_sample.txt")
        ).unwrap();
        assert_eq!(def.host, "10.0.1.50");
        assert_eq!(def.user, "admin");
    }

    #[test]
    fn parses_commands() {
        let def = SshSampleParser::parse(
            include_str!("../../tests/fixtures/ssh_sample.txt")
        ).unwrap();
        assert_eq!(def.commands.len(), 3);
    }

    #[test]
    fn extracts_parameters_from_template() {
        let def = SshSampleParser::parse(
            include_str!("../../tests/fixtures/ssh_sample.txt")
        ).unwrap();
        let cmd = def.commands.iter()
            .find(|c| c.command_template.contains("{interface}"))
            .unwrap();
        assert_eq!(cmd.parameters, vec!["interface"]);
    }

    #[test]
    fn preserves_sample_output() {
        let def = SshSampleParser::parse(
            include_str!("../../tests/fixtures/ssh_sample.txt")
        ).unwrap();
        let cmd = def.commands.iter()
            .find(|c| c.command_template.contains("interfaces"))
            .unwrap();
        assert!(cmd.sample_output.contains("Gi0/1"));
    }
}
