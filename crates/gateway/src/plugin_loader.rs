use api_anything_plugin_sdk::*;
use libloading::{Library, Symbol};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::adapter::{BoxFuture, ProtocolAdapter};
use crate::types::*;
use api_anything_common::error::AppError;

/// 已加载的动态插件实例
/// 持有 Library 引用以防止 .so/.dylib 被提前卸载（卸载后函数指针会悬空）
pub struct LoadedPlugin {
    _library: Library,
    info: PluginInfo,
    handle_fn: PluginHandleFn,
    free_fn: PluginFreeFn,
}

// Library 本身不是 Send+Sync，但我们只通过 C ABI 函数指针访问它，
// 函数指针在加载后是全局有效的，不涉及共享可变状态
unsafe impl Send for LoadedPlugin {}
unsafe impl Sync for LoadedPlugin {}

impl LoadedPlugin {
    pub fn info(&self) -> &PluginInfo {
        &self.info
    }

    /// 将请求序列化为 JSON 后通过 C ABI 传递给插件，再将响应 JSON 反序列化回来。
    /// 整个过程在 JSON 层面交互，使宿主和插件可以用不同编译器版本构建。
    pub fn handle(&self, request: &PluginRequest) -> Result<PluginResponse, PluginError> {
        let req_json = serde_json::to_string(request).map_err(|e| PluginError {
            code: 500,
            message: format!("Serialize error: {e}"),
        })?;
        let req_cstr = std::ffi::CString::new(req_json).unwrap();

        let resp_ptr = unsafe { (self.handle_fn)(req_cstr.as_ptr()) };
        let resp_cstr = unsafe { std::ffi::CStr::from_ptr(resp_ptr) };
        let resp_str = resp_cstr.to_str().map_err(|e| PluginError {
            code: 500,
            message: format!("UTF-8 error: {e}"),
        })?;

        let response: PluginResponse = serde_json::from_str(resp_str).map_err(|e| PluginError {
            code: 500,
            message: format!("Deserialize error: {e}"),
        })?;

        // 先完成反序列化再释放，因为 resp_str 借用了 resp_ptr 指向的内存
        unsafe { (self.free_fn)(resp_ptr) };
        Ok(response)
    }
}

/// 管理所有动态插件的生命周期：加载、查询、目录扫描
pub struct PluginManager {
    plugins: Arc<Mutex<HashMap<String, Arc<LoadedPlugin>>>>,
    plugin_dir: PathBuf,
}

impl PluginManager {
    pub fn new(plugin_dir: impl AsRef<Path>) -> Self {
        Self {
            plugins: Arc::new(Mutex::new(HashMap::new())),
            plugin_dir: plugin_dir.as_ref().to_path_buf(),
        }
    }

    /// 加载单个 .so/.dylib 插件文件
    /// 通过 libloading 解析三个必要的 C ABI 符号：plugin_info / plugin_handle / plugin_free
    pub fn load_plugin(&self, path: impl AsRef<Path>) -> Result<PluginInfo, anyhow::Error> {
        let path = path.as_ref();
        let lib = unsafe { Library::new(path)? };

        let info_fn: Symbol<PluginInfoFn> = unsafe { lib.get(b"plugin_info")? };
        let handle_fn: Symbol<PluginHandleFn> = unsafe { lib.get(b"plugin_handle")? };
        let free_fn: Symbol<PluginFreeFn> = unsafe { lib.get(b"plugin_free")? };

        // 获取插件自述信息
        let info_ptr = unsafe { info_fn() };
        let info_cstr = unsafe { std::ffi::CStr::from_ptr(info_ptr) };
        let info: PluginInfo = serde_json::from_str(info_cstr.to_str()?)?;

        // transmute 将绑定到 Symbol 生命周期的函数指针提升为 'static，
        // 安全性由 _library 字段保证——只要 LoadedPlugin 存活，.so 不会被卸载
        let handle_fn = unsafe { std::mem::transmute::<_, PluginHandleFn>(*handle_fn) };
        let free_fn_copy = unsafe { std::mem::transmute::<_, PluginFreeFn>(*free_fn) };

        // 用插件自己的 free 函数释放 info 字符串
        unsafe { free_fn_copy(info_ptr) };

        let plugin = Arc::new(LoadedPlugin {
            _library: lib,
            info: info.clone(),
            handle_fn,
            free_fn: free_fn_copy,
        });

        self.plugins
            .lock()
            .unwrap()
            .insert(info.name.clone(), plugin);
        tracing::info!(name = %info.name, version = %info.version, "Plugin loaded");
        Ok(info)
    }

    pub fn get_plugin(&self, name: &str) -> Option<Arc<LoadedPlugin>> {
        self.plugins.lock().unwrap().get(name).cloned()
    }

    pub fn list_plugins(&self) -> Vec<PluginInfo> {
        self.plugins
            .lock()
            .unwrap()
            .values()
            .map(|p| p.info.clone())
            .collect()
    }

    /// 扫描插件目录，自动加载所有 .so（Linux）和 .dylib（macOS）文件。
    /// 加载失败的文件会记录警告但不中断整体流程，保证部分插件损坏时系统仍可启动。
    pub fn scan_and_load(&self) -> Result<Vec<PluginInfo>, anyhow::Error> {
        let mut loaded = Vec::new();
        if !self.plugin_dir.exists() {
            return Ok(loaded);
        }
        for entry in std::fs::read_dir(&self.plugin_dir)? {
            let entry = entry?;
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext == "so" || ext == "dylib" {
                match self.load_plugin(&path) {
                    Ok(info) => loaded.push(info),
                    Err(e) => tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "Failed to load plugin"
                    ),
                }
            }
        }
        Ok(loaded)
    }
}

/// 将动态插件桥接为 ProtocolAdapter，使其可无缝接入网关的请求处理流水线
pub struct PluginAdapter {
    plugin: Arc<LoadedPlugin>,
}

impl PluginAdapter {
    pub fn new(plugin: Arc<LoadedPlugin>) -> Self {
        Self { plugin }
    }
}

impl ProtocolAdapter for PluginAdapter {
    fn transform_request(&self, req: &GatewayRequest) -> Result<BackendRequest, AppError> {
        // 插件在 execute 阶段统一处理请求转换和业务逻辑，
        // 这里仅做格式适配，将 GatewayRequest 映射到 BackendRequest
        Ok(BackendRequest {
            endpoint: self.plugin.info().name.clone(),
            method: req.method.clone(),
            headers: req.headers.clone(),
            body: req
                .body
                .as_ref()
                .map(|b| serde_json::to_vec(b).unwrap_or_default()),
            protocol_params: req.path_params.clone(),
        })
    }

    fn execute<'a>(
        &'a self,
        req: &'a BackendRequest,
    ) -> BoxFuture<'a, Result<BackendResponse, AppError>> {
        Box::pin(async move {
            let plugin_req = PluginRequest {
                method: req.method.to_string(),
                path: req.endpoint.clone(),
                headers: HashMap::new(),
                query_params: HashMap::new(),
                path_params: req.protocol_params.clone(),
                body: req
                    .body
                    .as_ref()
                    .and_then(|b| serde_json::from_slice(b).ok()),
            };

            let start = std::time::Instant::now();
            let resp = self.plugin.handle(&plugin_req).map_err(|e| {
                AppError::BackendError {
                    status: e.code,
                    detail: e.message,
                }
            })?;

            Ok(BackendResponse {
                status_code: resp.status_code,
                headers: axum::http::HeaderMap::new(),
                body: serde_json::to_vec(&resp.body).unwrap_or_default(),
                is_success: resp.status_code < 400,
                duration_ms: start.elapsed().as_millis() as u64,
            })
        })
    }

    fn transform_response(&self, resp: &BackendResponse) -> Result<GatewayResponse, AppError> {
        Ok(GatewayResponse {
            status_code: resp.status_code,
            headers: HashMap::new(),
            body: serde_json::from_slice(&resp.body).unwrap_or(serde_json::Value::Null),
        })
    }

    fn name(&self) -> &str {
        "plugin"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn plugin_manager_new_creates_instance() {
        let manager = PluginManager::new("/tmp/test-plugins");
        assert_eq!(manager.plugin_dir, PathBuf::from("/tmp/test-plugins"));
        assert!(manager.list_plugins().is_empty());
    }

    #[test]
    fn scan_and_load_returns_empty_for_nonexistent_dir() {
        let manager = PluginManager::new("/tmp/nonexistent-plugin-dir-12345");
        let result = manager.scan_and_load().unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn scan_and_load_returns_empty_for_empty_dir() {
        let dir = std::env::temp_dir().join("api-anything-test-empty-plugins");
        std::fs::create_dir_all(&dir).unwrap();
        let manager = PluginManager::new(&dir);
        let result = manager.scan_and_load().unwrap();
        assert!(result.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn scan_and_load_ignores_non_plugin_files() {
        let dir = std::env::temp_dir().join("api-anything-test-non-plugin-files");
        std::fs::create_dir_all(&dir).unwrap();
        // 创建一些非插件文件，确认它们被跳过
        std::fs::write(dir.join("readme.txt"), "not a plugin").unwrap();
        std::fs::write(dir.join("config.json"), "{}").unwrap();
        let manager = PluginManager::new(&dir);
        let result = manager.scan_and_load().unwrap();
        assert!(result.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn get_plugin_returns_none_for_unknown_name() {
        let manager = PluginManager::new("/tmp/test-plugins");
        assert!(manager.get_plugin("nonexistent").is_none());
    }
}
