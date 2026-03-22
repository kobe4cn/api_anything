use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// 插件元信息，宿主通过 plugin_info() 获取后据此注册路由和展示插件列表
#[repr(C)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
    pub protocol: String,
    pub description: String,
}

/// 宿主向插件传递的请求上下文，经 JSON 序列化跨 FFI 边界以规避 ABI 兼容性问题
#[derive(Debug, Serialize, Deserialize)]
pub struct PluginRequest {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub query_params: HashMap<String, String>,
    pub path_params: HashMap<String, String>,
    pub body: Option<Value>,
}

/// 插件处理后返回的响应，同样以 JSON 序列化跨 FFI 边界
#[derive(Debug, Serialize, Deserialize)]
pub struct PluginResponse {
    pub status_code: u16,
    pub headers: HashMap<String, String>,
    pub body: Value,
}

/// 插件错误，与 PluginResponse 区分以便宿主做统一的错误处理和日志记录
#[derive(Debug, Serialize, Deserialize)]
pub struct PluginError {
    pub code: u16,
    pub message: String,
}

/// 插件必须导出的 C ABI 函数签名
/// 所有数据通过 JSON 字符串（C 风格 *char）传递，避免跨 .so 边界的内存布局差异
pub type PluginInfoFn = unsafe extern "C" fn() -> *mut std::ffi::c_char;
pub type PluginHandleFn =
    unsafe extern "C" fn(request_json: *const std::ffi::c_char) -> *mut std::ffi::c_char;
pub type PluginFreeFn = unsafe extern "C" fn(ptr: *mut std::ffi::c_char);

/// 简化插件导出的宏：自动生成 plugin_info / plugin_handle / plugin_free 三个 C ABI 函数，
/// 插件开发者只需提供一个 handler 函数和 PluginInfo 实例即可
#[macro_export]
macro_rules! export_plugin {
    ($handler:expr, $info:expr) => {
        #[no_mangle]
        pub extern "C" fn plugin_info() -> *mut std::ffi::c_char {
            let info = serde_json::to_string(&$info).unwrap();
            std::ffi::CString::new(info).unwrap().into_raw()
        }

        #[no_mangle]
        pub extern "C" fn plugin_handle(
            request_json: *const std::ffi::c_char,
        ) -> *mut std::ffi::c_char {
            let req_str = unsafe { std::ffi::CStr::from_ptr(request_json) }
                .to_str()
                .unwrap();
            let request: $crate::PluginRequest = serde_json::from_str(req_str).unwrap();
            let response = $handler(request);
            let resp_str = serde_json::to_string(&response).unwrap();
            std::ffi::CString::new(resp_str).unwrap().into_raw()
        }

        #[no_mangle]
        pub extern "C" fn plugin_free(ptr: *mut std::ffi::c_char) {
            if !ptr.is_null() {
                unsafe {
                    drop(std::ffi::CString::from_raw(ptr));
                }
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_info_serialization_roundtrip() {
        let info = PluginInfo {
            name: "test-plugin".to_string(),
            version: "1.0.0".to_string(),
            protocol: "custom".to_string(),
            description: "A test plugin".to_string(),
        };
        let json = serde_json::to_string(&info).unwrap();
        let deserialized: PluginInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info.name, deserialized.name);
        assert_eq!(info.version, deserialized.version);
        assert_eq!(info.protocol, deserialized.protocol);
        assert_eq!(info.description, deserialized.description);
    }

    #[test]
    fn plugin_request_serialization_roundtrip() {
        let req = PluginRequest {
            method: "GET".to_string(),
            path: "/api/test".to_string(),
            headers: HashMap::from([("Content-Type".to_string(), "application/json".to_string())]),
            query_params: HashMap::from([("page".to_string(), "1".to_string())]),
            path_params: HashMap::new(),
            body: Some(serde_json::json!({"key": "value"})),
        };
        let json = serde_json::to_string(&req).unwrap();
        let deserialized: PluginRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req.method, deserialized.method);
        assert_eq!(req.path, deserialized.path);
        assert_eq!(req.body, deserialized.body);
    }

    #[test]
    fn plugin_response_serialization_roundtrip() {
        let resp = PluginResponse {
            status_code: 200,
            headers: HashMap::from([("X-Custom".to_string(), "value".to_string())]),
            body: serde_json::json!({"result": "ok"}),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: PluginResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp.status_code, deserialized.status_code);
        assert_eq!(resp.body, deserialized.body);
    }

    #[test]
    fn plugin_error_serialization_roundtrip() {
        let err = PluginError {
            code: 404,
            message: "Not found".to_string(),
        };
        let json = serde_json::to_string(&err).unwrap();
        let deserialized: PluginError = serde_json::from_str(&json).unwrap();
        assert_eq!(err.code, deserialized.code);
        assert_eq!(err.message, deserialized.message);
    }
}
