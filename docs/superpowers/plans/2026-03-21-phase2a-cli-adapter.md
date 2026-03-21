# Phase 2a: CLI 命令行包装适配器 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现 CLI 工具到 REST API 的自动包装 — 解析 CLI `--help` 输出提取命令结构，创建 CLI ProtocolAdapter（基于 tokio::process::Command），支持输出文本的 regex 解析转 JSON，并通过 CLI `generate` 命令端到端跑通。

**Architecture:** 在 generator crate 新增 CLI help 解析器和映射器（复用 UnifiedContract 中间表示）。在 gateway crate 新增 CLI ProtocolAdapter，通过 tokio::process 异步执行本地命令，强制使用 `.arg()` 传参防止命令注入。输出文本通过 regex 或 JSON 解析提取为结构化响应。CLI 后端默认并发上限 10（保护 OS 进程资源）。

**Tech Stack:** tokio::process, regex, 已有 gateway + generator crate

**Spec:** `docs/superpowers/specs/2026-03-21-api-anything-platform-design.md` §4, §5.4, §5.6

---

## File Structure

```
crates/generator/src/
    └── cli_help/
        ├── mod.rs
        ├── parser.rs               # --help 输出解析器
        └── mapper.rs               # CLI 解析结果 → UnifiedContract

crates/gateway/src/
    ├── adapters/
    │   └── cli_process.rs          # CLI ProtocolAdapter (tokio::process)
    └── output_parser.rs            # 命令输出文本 → JSON 转换
```

---

### Task 1: CLI Help 输出解析器

**Files:**
- Create: `crates/generator/src/cli_help/mod.rs`
- Create: `crates/generator/src/cli_help/parser.rs`
- Create: `crates/generator/tests/fixtures/sample_help.txt`

解析 CLI 工具的 `--help` 输出，提取命令名、子命令、选项、参数。

- [ ] **Step 1: 创建测试 fixture**

`sample_help.txt` — 模拟一个典型的 CLI 工具帮助输出：
```
report-gen 1.2.0
Report generation tool

USAGE:
    report-gen <SUBCOMMAND>

SUBCOMMANDS:
    generate    Generate a new report
    list        List existing reports
    export      Export report to file

OPTIONS:
    -h, --help       Print help information
    -V, --version    Print version
```

再创建 `sample_subcommand_help.txt` — 子命令的帮助：
```
report-gen generate
Generate a new report

USAGE:
    report-gen generate [OPTIONS] --type <TYPE>

OPTIONS:
    -t, --type <TYPE>        Report type (daily, weekly, monthly)
    -f, --format <FORMAT>    Output format [default: json] [possible values: json, csv, html]
    -s, --start <DATE>       Start date (YYYY-MM-DD)
    -e, --end <DATE>         End date (YYYY-MM-DD)
    -o, --output <PATH>      Output file path
    -h, --help               Print help information
```

- [ ] **Step 2: 定义解析结果类型**

```rust
#[derive(Debug, Clone)]
pub struct CliDefinition {
    pub program_name: String,
    pub description: String,
    pub subcommands: Vec<CliSubcommand>,
    pub global_options: Vec<CliOption>,
}

#[derive(Debug, Clone)]
pub struct CliSubcommand {
    pub name: String,
    pub description: String,
    pub options: Vec<CliOption>,
    pub positional_args: Vec<CliArg>,
}

#[derive(Debug, Clone)]
pub struct CliOption {
    pub short: Option<String>,   // -t
    pub long: Option<String>,    // --type
    pub value_name: Option<String>, // TYPE
    pub description: String,
    pub required: bool,
    pub default_value: Option<String>,
    pub possible_values: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CliArg {
    pub name: String,
    pub required: bool,
}
```

- [ ] **Step 3: 实现 CliHelpParser**

使用 regex 解析 `--help` 输出的常见格式（Clap/ArgParse 风格）：
- 提取程序名和描述（前几行）
- 匹配 `SUBCOMMANDS:` 段落提取子命令
- 匹配 `OPTIONS:` 段落提取选项（短名、长名、值名、描述、默认值、可选值）
- 检测 `[required]` 或必填标志

```rust
pub struct CliHelpParser;

impl CliHelpParser {
    /// 解析主命令的 --help 输出
    pub fn parse_main(help_text: &str) -> Result<CliDefinition, anyhow::Error> { ... }

    /// 解析子命令的 --help 输出，填充选项和参数
    pub fn parse_subcommand(help_text: &str) -> Result<CliSubcommand, anyhow::Error> { ... }
}
```

- [ ] **Step 4: 编写测试 (5 tests)**

```rust
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
        assert_eq!(def.subcommands.len(), 3); // generate, list, export
    }

    #[test]
    fn parses_subcommand_options() {
        let sub = CliHelpParser::parse_subcommand(include_str!("../../tests/fixtures/sample_subcommand_help.txt")).unwrap();
        assert!(sub.options.len() >= 4); // type, format, start, end, output (excluding help)
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
```

- [ ] **Step 5: 更新 generator lib.rs + Commit**

```bash
git commit -am "feat(generator): add CLI help output parser"
```

---

### Task 2: CLI → UnifiedContract 映射器

**Files:**
- Create: `crates/generator/src/cli_help/mapper.rs`

将 CliDefinition 确定性映射为 UnifiedContract。

- [ ] **Step 1: 实现映射规则**

每个子命令映射为一个 Operation：
- **HTTP 方法推断**：`list`/`show`/`get`/`status`/`info` → GET，`create`/`generate`/`add`/`new` → POST，`delete`/`remove`/`rm` → DELETE，`update`/`set`/`modify` → PUT，其他 → POST
- **路径**：`/api/v1/{program-name}/{subcommand-name}`
- **Request Body**：从 options 生成 JSON Schema（类型从 value_name 推断：DATE→string(date), PATH→string, NUMBER/COUNT/NUM→integer, 其他→string）
- **Response**：默认 `{"type": "object", "properties": {"stdout": {"type": "string"}, "exit_code": {"type": "integer"}}}`，如果输出格式选项含 `json` 则 response 为 `{"type": "object"}`（动态 JSON）

- [ ] **Step 2: 编写测试 (4 tests)**

```rust
#[test]
fn maps_subcommands_to_operations() { ... }

#[test]
fn infers_http_method_from_name() { ... }  // "list" → GET, "generate" → POST

#[test]
fn generates_request_schema_from_options() { ... }  // --type, --format → JSON Schema properties

#[test]
fn marks_required_options_in_schema() { ... }
```

- [ ] **Step 3: 更新 pipeline.rs 支持 CLI 输入**

添加 `run_cli` 方法：
```rust
pub async fn run_cli(
    repo: &impl MetadataRepo,
    project_id: Uuid,
    program_name: &str,
    main_help: &str,
    subcommand_helps: &[(&str, &str)], // (subcommand_name, help_text)
) -> Result<GenerationResult, anyhow::Error> { ... }
```

- [ ] **Step 4: Commit**

```bash
git commit -am "feat(generator): add CLI help to UnifiedContract mapper"
```

---

### Task 3: 命令输出解析器

**Files:**
- Create: `crates/gateway/src/output_parser.rs`

将命令行输出文本转换为结构化 JSON。

- [ ] **Step 1: 实现 OutputParser**

三种解析策略：
```rust
pub enum OutputFormat {
    /// 输出本身是 JSON，直接解析
    Json,
    /// 用 regex 从文本中提取字段
    Regex { patterns: Vec<RegexPattern> },
    /// 原样返回 stdout 文本
    RawText,
}

pub struct RegexPattern {
    pub field_name: String,
    pub pattern: String,  // 正则表达式，第一个捕获组为值
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
                            result.insert(p.field_name.clone(), Value::String(val.as_str().to_string()));
                        }
                    }
                }
                Ok(Value::Object(result))
            }
            OutputFormat::RawText => {
                Ok(json!({ "stdout": output }))
            }
        }
    }
}
```

- [ ] **Step 2: 编写测试 (4 tests)**

```rust
#[test]
fn parses_json_output() { ... }

#[test]
fn parses_with_regex_patterns() { ... }

#[test]
fn returns_raw_text_when_configured() { ... }

#[test]
fn handles_invalid_json_gracefully() { ... }
```

- [ ] **Step 3: 添加 regex 依赖到 gateway Cargo.toml**

```toml
regex = "1"
```

- [ ] **Step 4: Commit**

```bash
git commit -am "feat(gateway): add command output parser with JSON/regex/raw modes"
```

---

### Task 4: CLI ProtocolAdapter

**Files:**
- Create: `crates/gateway/src/adapters/cli_process.rs`
- Modify: `crates/gateway/src/adapters/mod.rs`

通过 tokio::process::Command 异步执行本地命令。

- [ ] **Step 1: 实现 CliAdapter**

```rust
use crate::adapter::{BoxFuture, ProtocolAdapter};
use crate::output_parser::{OutputFormat, OutputParser};
use crate::types::*;
use api_anything_common::error::AppError;
use axum::http::{HeaderMap, Method};
use std::collections::HashMap;
use std::time::Instant;
use tokio::process::Command;

#[derive(Debug, Clone)]
pub struct CliConfig {
    pub program: String,           // 可执行文件路径
    pub subcommand: Option<String>, // 子命令名（如 "generate"）
    pub static_args: Vec<String>,   // 固定参数（每次都附加）
    pub output_format: OutputFormat, // 输出解析方式
}

pub struct CliAdapter {
    config: CliConfig,
}

impl CliAdapter {
    pub fn new(config: CliConfig) -> Self { Self { config } }
}

impl ProtocolAdapter for CliAdapter {
    fn transform_request(&self, req: &GatewayRequest) -> Result<BackendRequest, AppError> {
        // 从 JSON body 构建命令行参数
        // 安全规则：所有参数通过 .arg() 传递，禁止字符串拼接
        let mut args = Vec::new();

        // 添加子命令
        if let Some(sub) = &self.config.subcommand {
            args.push(sub.clone());
        }

        // 添加固定参数
        args.extend(self.config.static_args.clone());

        // 将 JSON body 的字段转为 --key value 参数
        if let Some(body) = &req.body {
            if let Some(obj) = body.as_object() {
                for (key, val) in obj {
                    args.push(format!("--{}", key));
                    match val {
                        serde_json::Value::String(s) => args.push(s.clone()),
                        serde_json::Value::Bool(true) => {}, // flag 参数，无需 value
                        serde_json::Value::Bool(false) => { args.pop(); }, // 移除 flag
                        other => args.push(other.to_string()),
                    }
                }
            }
        }

        // 路径参数也转为命令行参数
        for (key, val) in &req.path_params {
            args.push(format!("--{}", key));
            args.push(val.clone());
        }

        let mut protocol_params = HashMap::new();
        protocol_params.insert("program".to_string(), self.config.program.clone());
        protocol_params.insert("args".to_string(), args.join(" "));

        Ok(BackendRequest {
            endpoint: self.config.program.clone(),
            method: Method::POST,
            headers: HeaderMap::new(),
            body: Some(serde_json::to_vec(&args).unwrap_or_default()),
            protocol_params,
        })
    }

    fn execute<'a>(&'a self, req: &'a BackendRequest) -> BoxFuture<'a, Result<BackendResponse, AppError>> {
        Box::pin(async move {
            let start = Instant::now();

            // 从 body 中提取参数列表
            let args: Vec<String> = req.body.as_ref()
                .and_then(|b| serde_json::from_slice(b).ok())
                .unwrap_or_default();

            // 安全执行：使用 .arg() 逐个传递，防止命令注入
            let mut cmd = Command::new(&req.endpoint);
            for arg in &args {
                cmd.arg(arg);
            }

            let output = cmd.output().await
                .map_err(|e| AppError::BackendUnavailable(format!("Failed to execute {}: {e}", req.endpoint)))?;

            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let exit_code = output.status.code().unwrap_or(-1);

            // 构建响应：exit_code + stdout + stderr
            let body = serde_json::json!({
                "exit_code": exit_code,
                "stdout": stdout,
                "stderr": stderr,
            });

            Ok(BackendResponse {
                status_code: if exit_code == 0 { 200 } else { 500 },
                headers: HeaderMap::new(),
                body: serde_json::to_vec(&body).unwrap_or_default(),
                // 仅依据 exit_code 判断，许多正常 CLI 工具会向 stderr 输出 warning
                is_success: exit_code == 0,
                duration_ms: start.elapsed().as_millis() as u64,
            })
        })
    }

    fn transform_response(&self, resp: &BackendResponse) -> Result<GatewayResponse, AppError> {
        let raw: serde_json::Value = serde_json::from_slice(&resp.body)
            .unwrap_or(serde_json::json!({}));

        let stdout = raw["stdout"].as_str().unwrap_or("");

        // 先检查 exit_code，避免命令失败时解析空 stdout 产生误导性错误
        let exit_code = raw["exit_code"].as_i64().unwrap_or(-1);
        if exit_code != 0 {
            let stderr = raw["stderr"].as_str().unwrap_or("Unknown error");
            return Err(AppError::BackendError {
                status: 500,
                detail: format!("Command failed (exit code {}): {}", exit_code, stderr),
            });
        }

        // 命令成功后再解析 stdout
        let parsed = OutputParser::parse(stdout, &self.config.output_format)?;

        Ok(GatewayResponse {
            status_code: 200,
            headers: HashMap::new(),
            body: parsed,
        })
    }

    fn name(&self) -> &str { "cli" }
}
```

- [ ] **Step 2: 编写测试 (3 tests)**

测试 transform_request（JSON→args 转换）和 execute（使用 `echo` 命令做端到端）：

```rust
#[test]
fn transforms_json_to_cli_args() { ... }

#[tokio::test]
async fn executes_echo_command() {
    // echo "hello" 作为最简测试
    let adapter = CliAdapter::new(CliConfig {
        program: "echo".to_string(),
        subcommand: None,
        static_args: vec!["hello".to_string()],
        output_format: OutputFormat::RawText,
    });
    // ... verify stdout contains "hello"
}

#[tokio::test]
async fn handles_nonexistent_command() {
    // 测试命令不存在时返回 BackendUnavailable
}
```

- [ ] **Step 3: 更新 adapters/mod.rs + gateway/lib.rs**

```rust
pub mod soap;
pub mod cli_process;
```

- [ ] **Step 4: Commit**

```bash
git commit -am "feat(gateway): add CLI protocol adapter with tokio::process"
```

---

### Task 5: RouteLoader 支持 CLI 协议 + CLI generate 扩展

**Files:**
- Modify: `crates/gateway/src/loader.rs` — 添加 CLI 协议分支
- Modify: `crates/cli/src/main.rs` — 添加 `generate-cli` 子命令
- Modify: `crates/generator/src/pipeline.rs` — 添加 `run_cli` 方法

- [ ] **Step 1: 扩展 RouteLoader**

在 loader.rs 的 match 中添加 CLI 分支：

```rust
ProtocolType::Cli => {
    let config = Self::build_cli_config(route)?;
    Box::new(CliAdapter::new(config))
}
```

`build_cli_config` 从 `route.endpoint_config` 提取 program、subcommand、static_args、output_format。

CLI 的默认保护策略比 SOAP 更保守。在 `build_protection_stack` 中根据 `route.protocol` 差异化默认值：
- CLI: max_concurrent=10, error_threshold=30%, window=10s, timeout=60s
- SOAP/HTTP: 保持原有默认值 (100/50%/30s/30s)

- [ ] **Step 2: 添加 CLI pipeline**

在 pipeline.rs 添加 `run_cli` 方法。接受主帮助和子命令帮助文本，解析后写入元数据。

- [ ] **Step 3: 添加 CLI 子命令到 CLI 工具**

```rust
/// Generate REST API from CLI tool help output
GenerateCli {
    /// Path to main help output text file
    #[arg(long)]
    main_help: String,
    /// Paths to subcommand help text files (name:path format)
    #[arg(long)]
    sub_helps: Vec<String>,
    /// Project name
    #[arg(short, long)]
    project: String,
    /// Path to the CLI executable
    #[arg(long)]
    program: String,
}
```

- [ ] **Step 4: 编写集成测试**

使用真实的 `echo` 或 `date` 命令验证 CLI 生成和代理流程。

- [ ] **Step 5: Commit**

```bash
git commit -am "feat: add CLI protocol support to route loader and generation pipeline"
```

---

### Task 6: 端到端验证

**Files:** 无新文件

- [ ] **Step 1: 准备 report-gen 模拟脚本**

创建 `crates/generator/tests/fixtures/mock-report-gen.sh`：
```bash
#!/bin/bash
case "$1" in
    generate)
        echo '{"report_id": "R-001", "status": "generated", "rows": 42}'
        ;;
    list)
        echo '[{"id": "R-001", "date": "2024-01-01"}, {"id": "R-002", "date": "2024-01-02"}]'
        ;;
    *)
        echo "Unknown subcommand: $1" >&2
        exit 1
        ;;
esac
```

- [ ] **Step 2: 生成 + 加载 + 代理**

1. 使用 sample_help.txt 运行 CLI 生成管道
2. RouteLoader 加载路由
3. POST `/gw/api/v1/report-gen/generate` → 返回 JSON report
4. GET `/gw/api/v1/report-gen/list` → 返回 JSON 列表

- [ ] **Step 3: 运行全量测试**

```bash
DATABASE_URL=... cargo test --workspace
```

- [ ] **Step 4: Commit**

```bash
git commit -am "test: add E2E test for CLI tool wrapping"
```

---

## Summary

| Task | 组件 | 产出 |
|------|------|------|
| 1 | CLI Help Parser | --help 输出解析器 + 5 测试 |
| 2 | CLI Mapper | CLI → UnifiedContract + HTTP 方法推断 + 4 测试 |
| 3 | Output Parser | JSON/regex/raw 三模式输出解析 + 4 测试 |
| 4 | CLI Adapter | tokio::process ProtocolAdapter + 3 测试 |
| 5 | 集成扩展 | RouteLoader CLI 分支 + pipeline + CLI 命令 |
| 6 | E2E | mock 脚本 → generate → load → proxy |

**Phase 2a 验收标准：** 使用模拟 CLI 工具的 --help 输出和 shell 脚本，CLI `generate-cli` 命令能完成全链路：解析帮助 → 映射为 REST 路由 → 写入元数据 → 网关加载路由 → JSON 请求触发命令执行 → 结果转为 JSON 返回。

**安全保障：** CLI 参数禁止字符串拼接，全部通过 `.arg()` 传递，从根源杜绝命令注入。
