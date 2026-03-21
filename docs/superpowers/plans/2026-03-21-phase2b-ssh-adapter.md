# Phase 2b: SSH 远程适配器 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现 SSH 远程命令到 REST API 的自动包装 — 解析 SSH 交互样例提取命令结构，创建 SSH ProtocolAdapter（基于 russh 纯 Rust SSH 客户端），支持 SSH 会话复用池，并通过端到端测试验证。

**Architecture:** 在 generator crate 新增 SSH 交互样例解析器。在 gateway crate 新增 SSH ProtocolAdapter，使用 russh（纯 Rust，无 C 依赖）实现异步 SSH 连接，通过会话池 (Arc<Mutex<Vec<Session>>>) 复用连接。复用 Phase 2a 的 OutputParser 解析远程命令输出。SSH 后端默认并发上限 5（保护远程服务器）。

**Tech Stack:** russh (纯 Rust SSH2 客户端), tokio, 已有 OutputParser

**Spec:** `docs/superpowers/specs/2026-03-21-api-anything-platform-design.md` §4, §5.4, §5.6

---

## File Structure

```
crates/generator/src/
    └── ssh_sample/
        ├── mod.rs
        ├── parser.rs               # SSH 交互样例解析器
        └── mapper.rs               # SSH 样例 → UnifiedContract

crates/gateway/src/
    └── adapters/
        └── ssh_remote.rs           # SSH ProtocolAdapter
```

---

### Task 1: SSH 交互样例解析器

**Files:**
- Create: `crates/generator/src/ssh_sample/mod.rs`
- Create: `crates/generator/src/ssh_sample/parser.rs`
- Create: `crates/generator/tests/fixtures/ssh_sample.txt`

用户提供 SSH 交互样例文本（手动编写或从终端复制），描述可执行的命令和期望输出格式。

- [ ] **Step 1: 创建样例 fixture**

`ssh_sample.txt` — 描述网络设备 SSH 命令的样例：
```
# SSH Interaction Sample
# Host: 10.0.1.50
# User: admin
# Description: Network switch management commands

## Command: show interfaces status
## Description: List all network interfaces with their status
## Output Format: table
## Sample Output:
Port      Name     Status    Speed    Duplex
Gi0/1     uplink   connected 1000M    full
Gi0/2     server1  connected 1000M    full
Gi0/3     -        notconnect auto    auto

## Command: show vlan brief
## Description: List VLAN configuration
## Output Format: table
## Sample Output:
VLAN  Name           Status    Ports
1     default        active    Gi0/1, Gi0/2
10    management     active    Gi0/3
20    production     active    Gi0/4, Gi0/5

## Command: show running-config interface {interface}
## Description: Show configuration for a specific interface
## Output Format: text
## Sample Output:
interface GigabitEthernet0/1
 description uplink
 switchport mode trunk
 speed 1000
 duplex full
```

- [ ] **Step 2: 定义解析结果类型**

```rust
#[derive(Debug, Clone)]
pub struct SshSampleDefinition {
    pub host: String,
    pub user: String,
    pub description: String,
    pub commands: Vec<SshCommand>,
}

#[derive(Debug, Clone)]
pub struct SshCommand {
    pub command_template: String,  // "show interfaces status" 或 "show running-config interface {interface}"
    pub description: String,
    pub output_format: String,     // "table", "json", "text"
    pub parameters: Vec<String>,   // 从 {param} 占位符提取
    pub sample_output: String,
}
```

- [ ] **Step 3: 实现解析器**

解析 `## Command:` / `## Description:` / `## Output Format:` / `## Sample Output:` 块，以及文件头部的 `# Host:` / `# User:` 元信息。

参数从命令模板的 `{param}` 占位符中提取。

- [ ] **Step 4: 编写测试 (4 tests)**

```rust
#[test] fn parses_host_and_user() { ... }
#[test] fn parses_commands() { ... }  // 3 commands
#[test] fn extracts_parameters_from_template() { ... }  // {interface} → ["interface"]
#[test] fn preserves_sample_output() { ... }
```

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(generator): add SSH interaction sample parser"
```

---

### Task 2: SSH → UnifiedContract 映射器

**Files:**
- Create: `crates/generator/src/ssh_sample/mapper.rs`

- [ ] **Step 1: 实现映射规则**

每个 SSH 命令映射为一个 Operation：
- **HTTP 方法**：`show`/`display`/`get`/`list` 开头 → GET，`config`/`set`/`enable`/`disable` 开头 → POST，其他 → POST
- **路径**：`/api/v1/{host-slug}/{command-slug}`，命令中的空格转 `-`，`{param}` 保留为路径参数
  - `show interfaces status` → `GET /api/v1/switch/show-interfaces-status`
  - `show running-config interface {interface}` → `GET /api/v1/switch/show-running-config-interface/{interface}`
- **Request Schema**：从 `{param}` 占位符生成路径参数，其他参数为空
- **endpoint_url**：`ssh://{user}@{host}`
- **endpoint_config** 包含：host, user, command_template, output_format

- [ ] **Step 2: 编写测试 (3 tests)**

```rust
#[test] fn maps_show_commands_to_get() { ... }
#[test] fn extracts_path_params() { ... }
#[test] fn generates_endpoint_config() { ... }
```

- [ ] **Step 3: 更新 pipeline.rs 添加 `run_ssh`**

- [ ] **Step 4: Commit**

```bash
git commit -am "feat(generator): add SSH sample to UnifiedContract mapper"
```

---

### Task 3: SSH ProtocolAdapter

**Files:**
- Create: `crates/gateway/src/adapters/ssh_remote.rs`
- Modify: `crates/gateway/src/adapters/mod.rs`

- [ ] **Step 1: 添加 russh 依赖**

在 workspace Cargo.toml 添加：
```toml
russh = "0.46"
russh-keys = "0.46"
async-trait = "0.1"
```

在 gateway Cargo.toml 添加这些依赖。

注意：如果 russh 版本不可用或编译有问题，可以降级使用 `tokio::process::Command` 包装系统 `ssh` 命令作为 fallback（类似 CLI adapter 的方式，调用 `ssh user@host "command"`）。这种方式更简单但依赖系统 ssh 客户端。

- [ ] **Step 2: 实现 SshAdapter**

```rust
#[derive(Debug, Clone)]
pub struct SshConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: SshAuth,
    pub command_template: String,
    pub output_format: OutputFormat,
}

#[derive(Debug, Clone)]
pub enum SshAuth {
    Password(String),
    Key { private_key_path: String },
}

pub struct SshAdapter {
    config: SshConfig,
}
```

**transform_request**：将路径参数替换到 command_template 的 `{param}` 占位符中，构建最终命令字符串。

**execute**：
- 方案 A（russh）：创建 SSH 连接 → 打开 channel → exec 命令 → 读取 stdout/stderr → 获取 exit status
- 方案 B（fallback）：使用 `tokio::process::Command::new("ssh")` 调用系统 ssh

由于 russh 的 API 可能不稳定，建议实现时先尝试方案 A，如果编译有问题则回退到方案 B。方案 B 的实现：

```rust
fn execute<'a>(&'a self, req: &'a BackendRequest) -> BoxFuture<'a, Result<BackendResponse, AppError>> {
    Box::pin(async move {
        let command = req.protocol_params.get("command").unwrap();
        let mut cmd = Command::new("ssh");
        cmd.arg("-o").arg("StrictHostKeyChecking=no")
           .arg("-o").arg("ConnectTimeout=10")
           .arg(&format!("{}@{}", self.config.user, self.config.host))
           .arg(command);

        // 密码认证需要 sshpass 或 key
        // Key 认证添加 -i 参数
        if let SshAuth::Key { private_key_path } = &self.config.auth {
            cmd.arg("-i").arg(private_key_path);
        }

        let output = cmd.output().await
            .map_err(|e| AppError::BackendUnavailable(format!("SSH failed: {e}")))?;
        // ... same stdout/stderr/exit_code handling as CLI adapter
    })
}
```

**transform_response**：复用 OutputParser（和 CLI adapter 一样）。

- [ ] **Step 3: 编写测试 (3 tests)**

单元测试（不需要真实 SSH 服务）：
```rust
#[test] fn transform_request_substitutes_params() { ... }
#[test] fn transform_request_preserves_static_command() { ... }
#[test] fn transform_response_parses_output() { ... }
```

- [ ] **Step 4: Commit**

```bash
git commit -am "feat(gateway): add SSH remote protocol adapter"
```

---

### Task 4: RouteLoader SSH 分支 + CLI 集成

**Files:**
- Modify: `crates/gateway/src/loader.rs`
- Modify: `crates/cli/src/main.rs`

- [ ] **Step 1: 添加 SSH 分支到 RouteLoader**

```rust
ProtocolType::Ssh => {
    let config = Self::build_ssh_config(route)?;
    Box::new(SshAdapter::new(config))
}
```

SSH 保护默认值（from spec §5.4）：并发 5、熔断 20%、超时 120s。这些在 Task 5 of Phase 2a 中已经通过 `build_protection_stack` 的协议感知默认值实现了。

- [ ] **Step 2: 添加 generate-ssh CLI 子命令**

```rust
GenerateSsh {
    /// Path to SSH interaction sample file
    #[arg(long)]
    sample: String,
    /// Project name
    #[arg(short, long)]
    project: String,
},
```

- [ ] **Step 3: 编写集成测试**

由于 SSH 测试需要真实服务器，编写一个 `#[ignore]` 标记的测试用于手动验证，以及一个不需要 SSH 的路由加载测试。

- [ ] **Step 4: 运行全量测试**

```bash
DATABASE_URL=... cargo test --workspace
```

- [ ] **Step 5: Commit**

```bash
git commit -am "feat: add SSH protocol support to route loader and CLI"
```

---

## Summary

| Task | 组件 | 产出 |
|------|------|------|
| 1 | SSH 样例解析器 | 交互样例文本解析 + 4 测试 |
| 2 | SSH Mapper | SSH → UnifiedContract + 3 测试 |
| 3 | SSH Adapter | ProtocolAdapter (ssh/系统ssh) + 3 测试 |
| 4 | 集成 | RouteLoader SSH 分支 + CLI 命令 |

**Phase 2b 验收标准：** SSH 交互样例文件可通过 `generate-ssh` 命令解析并生成路由。SSH 适配器可将命令模板中的参数替换并执行远程命令。RouteLoader 支持 SSH 协议路由加载。

**Phase 2 整体验收标准达成：** 能将 CLI 工具和 SSH 命令自动包装为 REST API。
