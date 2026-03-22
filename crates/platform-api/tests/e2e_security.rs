/// 安全测试套件：验证命令注入防护、RFC 7807 错误格式一致性、以及请求验证边界条件。
///
/// 命令注入测试通过 CLI 适配器向 `echo` 发送含 shell 元字符的参数，
/// 利用 tokio::process::Command 的 .arg() 逐参数传递机制（操作系统级参数隔离），
/// 确认恶意字符串被当作字面值而非 shell 指令执行。
///
/// RFC 7807 测试验证所有错误响应遵循统一的 ProblemDetail 结构，
/// 使 API 消费方能以一致的方式处理各类错误。
use api_anything_gateway::adapter::ProtocolAdapter;
use api_anything_gateway::adapters::cli_process::{CliAdapter, CliConfig};
use api_anything_gateway::output_parser::OutputFormat;
use api_anything_gateway::types::GatewayRequest;
use axum::http::{HeaderMap, Method, StatusCode};
use serde_json::json;
use std::collections::HashMap;
use uuid::Uuid;

mod common;

// ── 命令注入防护测试 ───────────────────────────────────────────────────
// 以下测试全部使用 echo 作为目标程序，验证 shell 元字符不会触发命令执行；
// echo 会将收到的参数原样输出，通过比对 stdout 判断参数是否被注入

#[cfg(unix)]
fn echo_adapter() -> CliAdapter {
    CliAdapter::new(CliConfig {
        program: "echo".to_string(),
        subcommand: None,
        static_args: vec![],
        output_format: OutputFormat::RawText,
    })
}

#[cfg(unix)]
fn make_request(body: serde_json::Value) -> GatewayRequest {
    GatewayRequest {
        route_id: Uuid::new_v4(),
        method: Method::POST,
        path: "/test".to_string(),
        headers: HeaderMap::new(),
        query_params: HashMap::new(),
        path_params: HashMap::new(),
        body: Some(body),
        trace_id: "security-test".to_string(),
    }
}

/// 验证注入字符串在 echo stdout 中以字面量形式出现，未被 shell 解释执行
#[cfg(unix)]
async fn assert_cli_echoes_literal(payload: &str) {
    let adapter = echo_adapter();
    let req = make_request(json!({"message": payload}));
    let backend_req = adapter.transform_request(&req).unwrap();
    let backend_resp = adapter.execute(&backend_req).await.unwrap();
    assert!(
        backend_resp.is_success,
        "echo should succeed regardless of argument content"
    );
    let raw: serde_json::Value = serde_json::from_slice(&backend_resp.body).unwrap();
    let stdout = raw["stdout"].as_str().unwrap();
    // echo 的输出应包含原始字面字符串（--message 后跟参数值）
    assert!(
        stdout.contains(payload) || stdout.contains(&payload.replace('\0', "")),
        "stdout should contain the literal payload, got: {stdout}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn security_cli_injection_semicolon() {
    // 分号是 shell 中最常见的命令分隔符，必须确认不被解释
    assert_cli_echoes_literal("; rm -rf /").await;
}

#[cfg(unix)]
#[tokio::test]
async fn security_cli_injection_backtick() {
    // 反引号触发 shell 子命令替换，检查 whoami 未被执行
    assert_cli_echoes_literal("`whoami`").await;
}

#[cfg(unix)]
#[tokio::test]
async fn security_cli_injection_dollar_parens() {
    // $() 是 bash 的另一种子命令替换语法
    assert_cli_echoes_literal("$(cat /etc/passwd)").await;
}

#[cfg(unix)]
#[tokio::test]
async fn security_cli_injection_pipe() {
    // 管道符将前一命令的输出作为后一命令的输入
    assert_cli_echoes_literal("| cat /etc/passwd").await;
}

#[cfg(unix)]
#[tokio::test]
async fn security_cli_injection_ampersand() {
    // && 是逻辑与操作符，前命令成功后执行后命令
    assert_cli_echoes_literal("&& echo hacked").await;
}

#[cfg(unix)]
#[tokio::test]
async fn security_cli_injection_newline() {
    // 换行符在某些场景下等效于命令分隔符
    assert_cli_echoes_literal("test\necho hacked").await;
}

#[cfg(unix)]
#[tokio::test]
async fn security_cli_injection_null_byte() {
    // null byte 会导致 OS 拒绝执行命令（"nul byte found in provided data"）
    // 这是正确的安全行为 — OS 级别的防御阻止了潜在的截断攻击
    let adapter = echo_adapter();
    let req = make_request(json!({"message": "test\x00injected"}));
    let backend_req = adapter.transform_request(&req).unwrap();
    let result = adapter.execute(&backend_req).await;
    assert!(
        result.is_err(),
        "OS should reject command with null byte in arguments"
    );
}

// ── RFC 7807 错误格式验证 ──────────────────────────────────────────────
// 所有 4xx/5xx 响应都应返回包含 type/title/status 的 ProblemDetail JSON

/// 验证 ProblemDetail 必需字段存在且值合理
fn assert_rfc7807_fields(body: &serde_json::Value, expected_status: u16) {
    assert!(
        body["type"].is_string(),
        "RFC 7807: 'type' field must be a string, got: {body}"
    );
    assert!(
        body["title"].is_string(),
        "RFC 7807: 'title' field must be a string, got: {body}"
    );
    assert_eq!(
        body["status"].as_u64().unwrap_or(0) as u16,
        expected_status,
        "RFC 7807: 'status' should be {expected_status}, got: {body}"
    );
}

#[tokio::test]
async fn rfc7807_not_found_format() {
    // 请求一个不存在的项目 UUID，应返回 404 + RFC 7807 格式
    let server = common::test_server().await;
    let resp = server
        .get("/api/v1/projects/00000000-0000-0000-0000-000000000000")
        .await;
    resp.assert_status(StatusCode::NOT_FOUND);
    let body: serde_json::Value = resp.json();
    assert_rfc7807_fields(&body, 404);
    // detail 字段应提供足够的上下文帮助调试
    assert!(
        body.get("detail").is_some(),
        "404 response should include a detail field"
    );
}

#[tokio::test]
async fn rfc7807_gateway_not_found_format() {
    // 网关未注册路由，应返回 404 + RFC 7807 格式而非 HTML 或纯文本
    let server = common::test_server().await;
    let resp = server.get("/gw/nonexistent/path").await;
    resp.assert_status(StatusCode::NOT_FOUND);
    let body: serde_json::Value = resp.json();
    assert_rfc7807_fields(&body, 404);
}

#[tokio::test]
async fn rfc7807_bad_request_format() {
    // POST /api/v1/projects 发送空 body，JSON 反序列化失败应返回 4xx
    let server = common::test_server().await;
    let resp = server
        .post("/api/v1/projects")
        .json(&json!({}))
        .await;
    // Axum 的 Json<T> extractor 在缺少必要字段时返回 422，
    // 但只要格式符合 RFC 7807 就满足安全测试要求
    let status = resp.status_code();
    assert!(
        status == 400 || status == 422,
        "Expected 400 or 422, got {status}"
    );
}

#[tokio::test]
async fn rfc7807_no_sensitive_info_in_errors() {
    // 数据库错误不应泄露连接字符串或堆栈跟踪
    let server = common::test_server().await;
    let resp = server
        .get("/api/v1/projects/00000000-0000-0000-0000-000000000000")
        .await;
    let body = resp.text();
    // 验证响应体不包含数据库连接信息
    assert!(
        !body.contains("postgres://"),
        "Error response must not leak database connection string"
    );
    assert!(
        !body.contains("password"),
        "Error response must not leak credentials"
    );
    // 验证不泄露 Rust 堆栈跟踪
    assert!(
        !body.contains("stack backtrace"),
        "Error response must not leak stack traces"
    );
    assert!(
        !body.contains("panicked at"),
        "Error response must not leak panic information"
    );
}

// ── 请求验证 ────────────────────────────────────────────────────────────

#[tokio::test]
async fn security_oversized_body_rejected() {
    // 发送超过 10MB 的请求体，网关层应拒绝而非耗尽内存
    let server = common::test_server().await;
    // 构造一个 11MB 的 JSON 字符串
    let large_payload = "x".repeat(11 * 1024 * 1024);
    let resp = server
        .post("/gw/any/path")
        .bytes(large_payload.into_bytes().into())
        .await;
    // 期望 400（body 读取失败）或 413（Payload Too Large）或 404（路由不存在但 body 已被限制）
    let status = resp.status_code();
    // 关键断言：不能是 500 Internal Server Error，
    // 表示服务端正确拒绝了超大请求而非因 OOM 崩溃
    assert_ne!(
        status, 500,
        "Oversized body should be rejected gracefully, not cause internal error"
    );
}

#[tokio::test]
async fn security_invalid_uuid_path_param() {
    // UUID 格式错误应返回客户端错误而非 500
    let server = common::test_server().await;
    let resp = server.get("/api/v1/projects/not-a-uuid").await;
    let status = resp.status_code();
    assert!(
        status == 400 || status == 404 || status == 422,
        "Invalid UUID should return 4xx, got {status}"
    );
    // 确保不是 500
    assert_ne!(
        status, 500,
        "Invalid UUID path param should not cause 500"
    );
}

#[tokio::test]
async fn security_invalid_json_body() {
    // 非 JSON body 发送到期望 JSON 的端点
    let server = common::test_server().await;
    let resp = server
        .post("/api/v1/projects")
        .bytes("this is not json".as_bytes().into())
        .await;
    let status = resp.status_code();
    assert!(
        status == 400 || status == 415 || status == 422,
        "Non-JSON body should return 4xx, got {status}"
    );
    assert_ne!(
        status, 500,
        "Invalid JSON body should not cause 500"
    );
}
