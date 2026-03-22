use std::fs;
use std::path::Path;

/// 创建临时 Rust crate 用于编译 LLM 生成的插件代码。
///
/// 生成的 Cargo.toml 通过绝对路径引用 plugin-sdk，
/// 避免 workspace_dir 位置变化导致相对路径失效。
pub fn create_plugin_crate(
    crate_dir: &Path,
    crate_name: &str,
    source_code: &str,
    sdk_path: &Path,
) -> Result<(), anyhow::Error> {
    fs::create_dir_all(crate_dir.join("src"))?;

    // 将 sdk_path 转为绝对路径字符串，用于 Cargo.toml 中的 path 依赖。
    // 使用绝对路径而非相对路径，使得 crate 可以放在任意位置而不影响依赖解析。
    let sdk_path_str = sdk_path
        .canonicalize()
        .unwrap_or_else(|_| sdk_path.to_path_buf())
        .display()
        .to_string();

    // [workspace] 空表使此 crate 独立于父 workspace，
    // 否则 Cargo 会认为它属于 api-anything 的 workspace 而报错
    let cargo_toml = format!(
        r#"[package]
name = "{crate_name}"
version = "0.1.0"
edition = "2021"

[workspace]

[lib]
crate-type = ["cdylib"]

[dependencies]
api-anything-plugin-sdk = {{ path = "{sdk_path_str}" }}
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
reqwest = {{ version = "0.12", features = ["json", "blocking"] }}
quick-xml = {{ version = "0.37", features = ["serialize"] }}
regex = "1"
tracing = "0.1"
"#
    );

    fs::write(crate_dir.join("Cargo.toml"), cargo_toml)?;
    fs::write(crate_dir.join("src/lib.rs"), source_code)?;

    Ok(())
}

/// 更新已存在 crate 的源码（编译失败后 LLM 修正代码时使用）
pub fn update_source_code(crate_dir: &Path, source_code: &str) -> Result<(), anyhow::Error> {
    fs::write(crate_dir.join("src/lib.rs"), source_code)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn creates_crate_directory_structure() {
        let dir = std::env::temp_dir().join("api-anything-test-scaffold");
        let _ = fs::remove_dir_all(&dir);

        let sdk_path = PathBuf::from("/tmp/fake-sdk");
        create_plugin_crate(&dir, "test-plugin", "// empty", &sdk_path).unwrap();

        assert!(dir.join("Cargo.toml").exists());
        assert!(dir.join("src/lib.rs").exists());

        let cargo_content = fs::read_to_string(dir.join("Cargo.toml")).unwrap();
        assert!(cargo_content.contains("test-plugin"));
        assert!(cargo_content.contains("cdylib"));
        assert!(cargo_content.contains("api-anything-plugin-sdk"));

        let source = fs::read_to_string(dir.join("src/lib.rs")).unwrap();
        assert_eq!(source, "// empty");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn update_source_overwrites_lib_rs() {
        let dir = std::env::temp_dir().join("api-anything-test-scaffold-update");
        let _ = fs::remove_dir_all(&dir);

        let sdk_path = PathBuf::from("/tmp/fake-sdk");
        create_plugin_crate(&dir, "test-plugin", "// v1", &sdk_path).unwrap();
        update_source_code(&dir, "// v2").unwrap();

        let source = fs::read_to_string(dir.join("src/lib.rs")).unwrap();
        assert_eq!(source, "// v2");

        let _ = fs::remove_dir_all(&dir);
    }
}
