use axum::http::{HeaderMap, Method};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct GatewayRequest {
    pub route_id: Uuid,
    pub method: Method,
    pub path: String,
    pub headers: HeaderMap,
    pub query_params: HashMap<String, String>,
    pub path_params: HashMap<String, String>,
    pub body: Option<Value>,
    pub trace_id: String,
}

#[derive(Debug, Clone)]
pub struct BackendRequest {
    pub endpoint: String,
    pub method: Method,
    pub headers: HeaderMap,
    pub body: Option<Vec<u8>>,
    pub protocol_params: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct BackendResponse {
    pub status_code: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    pub is_success: bool,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayResponse {
    pub status_code: u16,
    pub headers: HashMap<String, String>,
    pub body: Value,
}
