use crate::types::{BackendRequest, BackendResponse, GatewayRequest, GatewayResponse};
use api_anything_common::error::AppError;
use std::future::Future;
use std::pin::Pin;

// 使用 BoxFuture 而非 impl Future，使 trait 支持 dyn dispatch（动态分发）；
// impl Future 在 trait 中会导致 dyn 不兼容，无法装入 Box<dyn ProtocolAdapter>
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait ProtocolAdapter: Send + Sync {
    fn transform_request(&self, req: &GatewayRequest) -> Result<BackendRequest, AppError>;

    fn execute<'a>(&'a self, req: &'a BackendRequest) -> BoxFuture<'a, Result<BackendResponse, AppError>>;

    fn transform_response(&self, resp: &BackendResponse) -> Result<GatewayResponse, AppError>;

    fn name(&self) -> &str;
}
