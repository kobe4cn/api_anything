use api_anything_plugin_sdk::*;
use serde::{Serialize, Deserialize};
use reqwest::blocking::Client;
use regex::Regex;
use tracing;
use std::collections::HashMap;

// REST API Request/Response structs
#[derive(Serialize, Deserialize)]
struct AddRequest {
    a: i32,
    b: i32,
}

#[derive(Serialize, Deserialize)]
struct AddResponse {
    result: i32,
}

#[derive(Serialize, Deserialize)]
struct GetHistoryRequest {
    limit: i32,
}

#[derive(Serialize, Deserialize)]
struct GetHistoryResponse {
    entries: Vec<String>,
}

// SOAP Fault response struct
#[derive(Serialize, Deserialize)]
struct SoapErrorResponse {
    fault_code: String,
    fault_string: String,
}

// Constants from WSDL
const SOAP_ENDPOINT: &str = "http://example.com/calculator";
const NAMESPACE: &str = "http://example.com/calculator";

fn build_soap_envelope(body_content: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/" xmlns:tns="{}">
  <soap:Body>
    {}
  </soap:Body>
</soap:Envelope>"#,
        NAMESPACE, body_content
    )
}

fn send_soap_request(soap_action: &str, body: &str) -> Result<String, String> {
    let client = Client::new();
    
    let envelope = build_soap_envelope(body);
    
    let response = client
        .post(SOAP_ENDPOINT)
        .header("Content-Type", "text/xml; charset=utf-8")
        .header("SOAPAction", soap_action)
        .body(envelope)
        .send()
        .map_err(|e| format!("HTTP request failed: {}", e))?;
    
    let status = response.status();
    let text = response.text().map_err(|e| format!("Failed to read response: {}", e))?;
    
    if !status.is_success() {
        return Err(format!("HTTP error {}: {}", status, text));
    }
    
    Ok(text)
}

fn parse_soap_fault(xml: &str) -> Option<SoapErrorResponse> {
    let faultcode_re = Regex::new(r"<faultcode[^>]*>(?:\s*)([^<]*)(?:\s*)</faultcode>").ok()?;
    let faultstring_re = Regex::new(r"<faultstring[^>]*>(?:\s*)([^<]*)(?:\s*)</faultstring>").ok()?;
    
    let faultcode = faultcode_re
        .captures(xml)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().trim().to_string())
        .unwrap_or_else(|| "Unknown".to_string());
    
    let fault_string = faultstring_re
        .captures(xml)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().trim().to_string())
        .unwrap_or_else(|| "Unknown error".to_string());
    
    // Check if we actually found a fault
    if xml.contains("<faultcode>") || xml.contains("<Fault>") || xml.contains(":Fault") {
        Some(SoapErrorResponse {
            fault_code: faultcode,
            fault_string,
        })
    } else {
        None
    }
}

fn extract_xml_value(xml: &str, tag: &str) -> Option<String> {
    let pattern = format!(r"<{}[^>]*>([^<]*)</{}>", tag, tag);
    let re = Regex::new(&pattern).ok()?;
    re.captures(xml)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().trim().to_string())
}

fn extract_xml_values(xml: &str, tag: &str) -> Vec<String> {
    let pattern = format!(r"<{}[^>]*>([^<]*)</{}>", tag, tag);
    match Regex::new(&pattern) {
        Ok(re) => re
            .captures_iter(xml)
            .filter_map(|caps| caps.get(1))
            .map(|m| m.as_str().trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn error_response(status_code: u16, message: String) -> PluginResponse {
    PluginResponse {
        status_code,
        body: serde_json::json!({ "error": message }),
        headers: HashMap::new(),
    }
}

fn success_response<T: Serialize>(data: T) -> PluginResponse {
    PluginResponse {
        status_code: 200,
        body: serde_json::to_value(&data).unwrap_or(serde_json::json!({})),
        headers: {
            let mut headers = HashMap::new();
            headers.insert("content-type".to_string(), "application/json".to_string());
            headers
        },
    }
}

fn handle_add(req: &PluginRequest) -> PluginResponse {
    // Parse JSON request - body is Option<Value>
    let body_value = match &req.body {
        Some(v) => v,
        None => return error_response(400, "Missing request body".to_string()),
    };
    
    let add_req: AddRequest = match serde_json::from_value(body_value.clone()) {
        Ok(r) => r,
        Err(e) => return error_response(400, format!("Invalid JSON request: {}", e)),
    };
    
    // Build SOAP body
    let soap_body = format!(
        r#"<tns:AddRequest>
      <tns:a>{}</tns:a>
      <tns:b>{}</tns:b>
    </tns:AddRequest>"#,
        add_req.a, add_req.b
    );
    
    // Send SOAP request
    let soap_action = format!("{}/Add", NAMESPACE);
    let soap_response = match send_soap_request(&soap_action, &soap_body) {
        Ok(r) => r,
        Err(e) => return error_response(500, e),
    };
    
    // Check for SOAP fault
    if let Some(fault) = parse_soap_fault(&soap_response) {
        return PluginResponse {
            status_code: 500,
            body: serde_json::to_value(&fault).unwrap_or(serde_json::json!({ "error": soap_response.clone() })),
            headers: {
                let mut headers = HashMap::new();
                headers.insert("content-type".to_string(), "application/json".to_string());
                headers
            },
        };
    }
    
    // Extract result from response
    let result = match extract_xml_value(&soap_response, "result") {
        Some(val) => match val.parse::<i32>() {
            Ok(n) => n,
            Err(_) => return error_response(500, "Failed to parse result value".to_string()),
        },
        None => return error_response(500, "No result found in SOAP response".to_string()),
    };
    
    let response = AddResponse { result };
    success_response(response)
}

fn handle_get_history(req: &PluginRequest) -> PluginResponse {
    // Parse JSON request - body is Option<Value>
    let body_value = match &req.body {
        Some(v) => v,
        None => return error_response(400, "Missing request body".to_string()),
    };
    
    let history_req: GetHistoryRequest = match serde_json::from_value(body_value.clone()) {
        Ok(r) => r,
        Err(e) => return error_response(400, format!("Invalid JSON request: {}", e)),
    };
    
    // Build SOAP body
    let soap_body = format!(
        r#"<tns:GetHistoryRequest>
      <tns:limit>{}</tns:limit>
    </tns:GetHistoryRequest>"#,
        history_req.limit
    );
    
    // Send SOAP request
    let soap_action = format!("{}/GetHistory", NAMESPACE);
    let soap_response = match send_soap_request(&soap_action, &soap_body) {
        Ok(r) => r,
        Err(e) => return error_response(500, e),
    };
    
    // Check for SOAP fault
    if let Some(fault) = parse_soap_fault(&soap_response) {
        return PluginResponse {
            status_code: 500,
            body: serde_json::to_value(&fault).unwrap_or(serde_json::json!({ "error": soap_response.clone() })),
            headers: {
                let mut headers = HashMap::new();
                headers.insert("content-type".to_string(), "application/json".to_string());
                headers
            },
        };
    }
    
    // Extract entries from response
    let entries = extract_xml_values(&soap_response, "entries");
    
    let response = GetHistoryResponse { entries };
    success_response(response)
}

#[tracing::instrument(skip(req))]
fn handle(req: PluginRequest) -> PluginResponse {
    // Normalize path (remove trailing slash, ensure leading slash)
    let path = req.path.trim_end_matches('/');
    
    match path {
        "/add" => handle_add(&req),
        "/gethistory" | "/get-history" | "/GetHistory" => handle_get_history(&req),
        "/" => error_response(400, "Missing operation path. Use /add or /gethistory".to_string()),
        _ => error_response(404, format!("Unknown endpoint: {}. Available: /add, /gethistory", path)),
    }
}

export_plugin!(handle, PluginInfo {
    name: "calculator-soap".to_string(),
    version: "1.0.0".to_string(),
    protocol: "soap".to_string(),
    description: "SOAP Calculator service - converts SOAP operations to REST endpoints".to_string(),
});