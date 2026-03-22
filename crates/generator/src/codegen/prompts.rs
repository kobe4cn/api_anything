/// 系统 prompt：定义 LLM 的角色和输出格式要求。
/// 明确列出可用依赖，避免 LLM 引入项目中不存在的 crate 导致编译失败
pub const SYSTEM_PROMPT: &str = r#"
You are an expert Rust code generator for the API-Anything platform.
Your job is to generate a complete Rust plugin (.so dynamic library) that converts
a legacy system interface into a modern REST API.

The plugin must:
1. Use the api_anything_plugin_sdk crate (PluginInfo, PluginRequest, PluginResponse, export_plugin!)
2. Implement a handler function: fn handle(req: PluginRequest) -> PluginResponse
3. Define proper serde structs for request/response types
4. Handle errors gracefully, never panic
5. Include #[tracing::instrument] on the handler function (use tracing crate)

IMPORTANT CONSTRAINTS:
- The handler runs in a synchronous FFI context (extern "C"). You CANNOT use async/.await directly.
- For HTTP calls, use reqwest::blocking::Client (the "blocking" feature is enabled).
- Never use tokio::runtime::Runtime inside the handler — just use blocking calls.
- All plugin_handle calls receive JSON-serialized PluginRequest and must return JSON-serialized PluginResponse.

Output ONLY valid Rust code in a single ```rust code block. No explanations outside the code block.

The code must compile with these dependencies (and ONLY these):
- api_anything_plugin_sdk (provides: PluginInfo, PluginRequest, PluginResponse, export_plugin!)
- serde, serde_json (for serialization, with "derive" feature)
- reqwest (with "json" and "blocking" features, for HTTP calls)
- quick-xml (with "serialize" feature, for XML parsing if needed)
- regex (for text parsing)
- tracing (for observability)
"#;

/// 根据接口类型构建代码生成 prompt，每种接口类型使用专门优化的 prompt 模板
pub fn build_codegen_prompt(interface_type: &str, input_content: &str) -> String {
    match interface_type {
        "soap" | "wsdl" => build_soap_prompt(input_content),
        "odata" => build_odata_prompt(input_content),
        "openapi" | "rest" => build_openapi_prompt(input_content),
        "cli" => build_cli_prompt(input_content),
        "ssh" => build_ssh_prompt(input_content),
        "pty" => build_pty_prompt(input_content),
        _ => format!(
            "Generate a REST API plugin for the following interface:\n\n{}",
            input_content
        ),
    }
}

fn build_soap_prompt(wsdl_content: &str) -> String {
    format!(
        r#"
Generate a Rust plugin that converts the following SOAP/WSDL service into REST API endpoints.

WSDL Definition:
```xml
{wsdl_content}
```

Requirements:
1. Parse the WSDL and create typed request/response structs with serde Serialize/Deserialize
2. For each SOAP operation, the handler should:
   - Accept JSON request body
   - Build a SOAP XML envelope with the correct namespace and SOAPAction
   - Send HTTP POST to the SOAP endpoint using reqwest::blocking::Client
   - Parse the SOAP XML response back to JSON
   - Return the structured JSON response
3. Map XSD types to Rust types: xsd:int->i32, xsd:string->String, xsd:boolean->bool, xsd:float->f64
4. Handle SOAP Faults by returning PluginResponse with appropriate error status
5. Set the PluginInfo with protocol="soap"
6. The handler function signature must be: fn handle(req: PluginRequest) -> PluginResponse
7. Route requests based on req.path (e.g., "/add" for an Add operation)

Example structure:
```rust
use api_anything_plugin_sdk::*;
use serde::{{Serialize, Deserialize}};

#[derive(Serialize, Deserialize)]
struct AddRequest {{ a: i32, b: i32 }}

#[derive(Serialize, Deserialize)]
struct AddResponse {{ result: i32 }}

#[tracing::instrument(skip(req))]
fn handle(req: PluginRequest) -> PluginResponse {{
    // Route based on req.path
    // Build SOAP envelope, send request, parse response
    // Return PluginResponse
}}

export_plugin!(handle, PluginInfo {{
    name: "calculator-soap".to_string(),
    version: "1.0.0".to_string(),
    protocol: "soap".to_string(),
    description: "SOAP Calculator service".to_string(),
}});
```
"#
    )
}

fn build_odata_prompt(metadata_content: &str) -> String {
    format!(
        r#"
Generate a Rust plugin that converts the following OData service ($metadata) into REST API endpoints.

OData $metadata:
```xml
{metadata_content}
```

Requirements:
1. Parse EntityTypes and create typed Rust structs with serde
2. For each EntitySet, implement CRUD operations:
   - GET /entityset -> list entities (support $filter, $select, $top, $skip via query_params)
   - GET /entityset/{{key}} -> get single entity
   - POST /entityset -> create entity
   - PATCH /entityset/{{key}} -> update entity
   - DELETE /entityset/{{key}} -> delete entity
3. Forward OData query parameters to the backend OData service using reqwest::blocking::Client
4. Parse OData JSON responses and return them
5. Set PluginInfo with protocol="odata"
6. Handle OData error responses (error.code, error.message)
7. Route based on req.method and req.path
"#
    )
}

fn build_openapi_prompt(spec_content: &str) -> String {
    format!(
        r#"
Generate a Rust plugin that proxies the following OpenAPI/REST service.

OpenAPI Spec:
```
{spec_content}
```

Requirements:
1. For each path/operation in the spec, create typed request/response structs
2. Forward requests to the backend with proper method, headers, and body using reqwest::blocking::Client
3. Transform responses if needed (field mapping, type conversion)
4. Set PluginInfo with protocol="rest"
5. Route based on req.method and req.path
"#
    )
}

fn build_cli_prompt(help_content: &str) -> String {
    format!(
        r#"
Generate a Rust plugin that wraps the following CLI tool as REST API endpoints.

CLI Help Output:
```
{help_content}
```

Requirements:
1. For each subcommand, create a REST endpoint routed by req.path
2. Map JSON body fields to command-line arguments (--key value) safely
3. Execute the command using std::process::Command with .arg() (NEVER use shell string concatenation)
4. Parse stdout output:
   - If output looks like JSON, parse it directly with serde_json::from_str
   - If output is tabular/text, use regex to extract structured data into JSON
5. Map exit codes: 0->200, non-zero->500 with stderr in error detail
6. Set PluginInfo with protocol="cli"
7. Sanitize all input parameters before passing to Command (reject shell metacharacters)
"#
    )
}

fn build_ssh_prompt(sample_content: &str) -> String {
    format!(
        r#"
Generate a Rust plugin that wraps the following SSH remote commands as REST API endpoints.

SSH Interaction Sample:
```
{sample_content}
```

Requirements:
1. For each command in the sample, create a REST endpoint routed by req.path
2. Execute commands via std::process::Command::new("ssh") with proper args
3. Support {{{{param}}}} template variables from req.body or req.path_params
4. Parse command output (table/text/JSON) into structured JSON
5. Handle SSH errors: exit 255->502 (connection error), other non-zero->500
6. Set PluginInfo with protocol="ssh"
7. Use -o BatchMode=yes and -o ConnectTimeout=10 for non-interactive, timeout-safe execution
"#
    )
}

fn build_pty_prompt(recording_content: &str) -> String {
    format!(
        r#"
Generate a Rust plugin that wraps the following interactive terminal session as REST API endpoints.

PTY Interaction Recording:
```
{recording_content}
```

Requirements:
1. For each command sequence, create a REST endpoint routed by req.path
2. Execute via std::process::Command with piped stdin/stdout
3. Implement expect-style interaction: write command to stdin -> read stdout until prompt -> collect output
4. Use regex for prompt detection
5. Handle timeouts gracefully (set a deadline, return 504 on timeout)
6. Set PluginInfo with protocol="pty"
"#
    )
}

/// 构建测试代码生成 prompt
pub fn build_test_prompt(source_code: &str) -> String {
    format!(
        r#"
Generate comprehensive Rust test code for the following plugin:

```rust
{source_code}
```

Generate tests that:
1. Test each endpoint/operation with valid input
2. Test error handling (invalid input, missing fields)
3. Test edge cases (empty body, special characters)
4. Use serde_json::json! to construct test PluginRequest inputs
5. Assert on PluginResponse status_code and body structure
6. Tests should call the handler function directly (not via FFI)

Output ONLY the test code in a ```rust code block. Include #[cfg(test)] module.
"#
    )
}

/// 构建路由提取 prompt，让 LLM 从生成的 Rust 代码中识别出所有可用路由
pub fn build_routes_prompt(source_code: &str) -> String {
    format!(
        r#"
Analyze the following Rust plugin code and extract all API routes it handles.

```rust
{source_code}
```

Return a JSON array where each element has:
- "method": HTTP method (GET, POST, PUT, DELETE, PATCH)
- "path": REST path (e.g., "/api/v1/calculator/add")
- "name": operation name (camelCase)
- "description": brief description of what this endpoint does
- "request_schema": JSON Schema for request body (or {{}} if no body)
- "response_schema": JSON Schema for successful response body

Output ONLY the JSON array. No markdown fences, no explanation text.
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_codegen_prompt_routes_soap() {
        let prompt = build_codegen_prompt("soap", "<definitions/>");
        assert!(prompt.contains("SOAP"));
        assert!(prompt.contains("reqwest::blocking"));
    }

    #[test]
    fn build_codegen_prompt_routes_cli() {
        let prompt = build_codegen_prompt("cli", "usage: tool [command]");
        assert!(prompt.contains("CLI"));
        assert!(prompt.contains("std::process::Command"));
    }

    #[test]
    fn build_codegen_prompt_routes_ssh() {
        let prompt = build_codegen_prompt("ssh", "ssh user@host ls -la");
        assert!(prompt.contains("SSH"));
        assert!(prompt.contains("BatchMode"));
    }

    #[test]
    fn build_codegen_prompt_routes_unknown_type() {
        let prompt = build_codegen_prompt("unknown", "some content");
        assert!(prompt.contains("some content"));
    }

    #[test]
    fn system_prompt_mentions_blocking() {
        // 确保系统 prompt 明确告知 LLM 使用 blocking 调用，避免生成 async 代码
        assert!(SYSTEM_PROMPT.contains("blocking"));
        assert!(SYSTEM_PROMPT.contains("synchronous"));
    }

    #[test]
    fn test_prompt_requests_cfg_test() {
        let prompt = build_test_prompt("fn handle(req: PluginRequest) -> PluginResponse {}");
        assert!(prompt.contains("#[cfg(test)]"));
    }

    #[test]
    fn routes_prompt_requests_json_array() {
        let prompt = build_routes_prompt("fn handle(req: PluginRequest) -> PluginResponse {}");
        assert!(prompt.contains("JSON array"));
        assert!(prompt.contains("method"));
        assert!(prompt.contains("path"));
    }
}
