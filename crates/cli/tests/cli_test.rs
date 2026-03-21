use assert_cmd::Command;
use predicates::prelude::*;
use std::env;

fn cli() -> Command {
    let mut cmd = Command::cargo_bin("api-anything-cli").unwrap();
    let database_url = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://api_anything:api_anything@localhost:5432/api_anything".to_string());
    cmd.env("DATABASE_URL", database_url);
    cmd
}

#[test]
fn shows_help() {
    cli()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("API gateway generator"));
}

#[test]
fn generate_requires_source_and_project() {
    cli()
        .arg("generate")
        .assert()
        .failure()
        .stderr(predicate::str::contains("--source"));
}

#[test]
fn generate_fails_on_missing_file() {
    cli()
        .args(["generate", "--source", "nonexistent.wsdl", "--project", "test-missing"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Failed to read"));
}

#[test]
fn generate_succeeds_with_valid_wsdl() {
    let project_name = format!("cli-test-{}", uuid::Uuid::new_v4());
    let wsdl_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../generator/tests/fixtures/calculator.wsdl");

    cli()
        .args(["generate", "--source", wsdl_path, "--project", &project_name])
        .assert()
        .success()
        .stdout(predicate::str::contains("Generation complete!"))
        .stdout(predicate::str::contains("Routes created: 2"));

    // 清理生成的 OpenAPI spec 文件
    let spec_path = format!("{}.openapi.json", wsdl_path);
    let _ = std::fs::remove_file(&spec_path);
}
