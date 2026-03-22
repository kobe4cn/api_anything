use std::path::{Path, PathBuf};
use std::process::Command;

use crate::codegen::{extract_rust_code, sanitize_rust_code};
use crate::codegen::scaffold;
use crate::llm::client::LlmClient;

/// 编译生成的插件 crate 为动态链接库（Linux: .so, macOS: .dylib）。
///
/// 使用 --release 构建以获得更小的产物和更好的运行时性能。
pub fn compile_plugin(crate_dir: &Path) -> Result<PathBuf, anyhow::Error> {
    tracing::info!(crate_dir = %crate_dir.display(), "Compiling plugin crate");

    let output = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .current_dir(crate_dir)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("Plugin compilation failed:\n{}", stderr));
    }

    find_compiled_artifact(crate_dir)
}

/// 在 target/release 目录下查找编译产物。
/// crate 名称中的连字符在编译时被替换为下划线，需要做此映射。
fn find_compiled_artifact(crate_dir: &Path) -> Result<PathBuf, anyhow::Error> {
    let target_dir = crate_dir.join("target/release");
    let crate_name = crate_dir
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .replace('-', "_");

    let so_path = if cfg!(target_os = "macos") {
        target_dir.join(format!("lib{}.dylib", crate_name))
    } else {
        target_dir.join(format!("lib{}.so", crate_name))
    };

    if so_path.exists() {
        tracing::info!(path = %so_path.display(), "Plugin compiled successfully");
        Ok(so_path)
    } else {
        Err(anyhow::anyhow!(
            "Compiled plugin not found at {}",
            so_path.display()
        ))
    }
}

/// 编译生成的插件，失败时将编译错误发送给 LLM 请求修正代码。
///
/// 最多重试 max_retries 次。每次失败后将完整的错误信息连同原始代码一起发给 LLM，
/// 让其生成修正后的完整代码（而非 diff），确保修正后的代码可以直接替换编译。
/// 返回 (编译产物路径, 最终通过编译的源码)。
pub async fn compile_with_llm_fix(
    crate_dir: &Path,
    source_code: &str,
    llm: &dyn LlmClient,
    max_retries: u32,
) -> Result<(PathBuf, String), anyhow::Error> {
    let mut current_code = source_code.to_string();

    for attempt in 0..=max_retries {
        scaffold::update_source_code(crate_dir, &current_code)?;

        match compile_plugin(crate_dir) {
            Ok(path) => return Ok((path, current_code)),
            Err(e) if attempt < max_retries => {
                tracing::warn!(
                    attempt = attempt + 1,
                    max_retries,
                    "Compilation failed, asking LLM to fix"
                );

                let fix_prompt = format!(
                    r#"The following Rust plugin code failed to compile. Fix ALL errors and return ONLY the complete corrected Rust source code.

CRITICAL RULES:
1. Return ONLY Rust code - NO explanations, NO natural language text
2. The code must be wrapped in ```rust ... ``` markers
3. Do NOT include any text before or after the code block
4. Do NOT add comments like "Here's the fix" or "The issue was..."
5. The entire response must be valid Rust code
6. Keep all existing functionality, only fix the compilation errors
7. Make sure all strings use ASCII quotes (not Unicode smart quotes)

Failed code:
```rust
{}
```

Compilation errors:
```
{}
```

Return the COMPLETE fixed code (not just the changed parts):"#,
                    current_code, e
                );

                let fixed = llm
                    .complete(
                        "You are a Rust expert. Fix compilation errors in plugin code. \
                         Return ONLY the complete fixed code in a ```rust code block. \
                         NO explanations, NO text outside the code block.",
                        &fix_prompt,
                    )
                    .await?;

                // 提取代码块后再清洗，去除 LLM 可能附带的自然语言和 Unicode 特殊字符
                current_code = sanitize_rust_code(&extract_rust_code(&fixed));
            }
            Err(e) => return Err(e),
        }
    }

    Err(anyhow::anyhow!(
        "Failed to compile after {} retries",
        max_retries
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_artifact_returns_error_for_nonexistent_dir() {
        let path = Path::new("/tmp/nonexistent-crate-dir-12345");
        let result = find_compiled_artifact(path);
        assert!(result.is_err());
    }
}
