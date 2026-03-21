use crate::types::{BackendRequest, BackendResponse, GatewayRequest, GatewayResponse};
use api_anything_common::error::AppError;

pub trait ProtocolAdapter: Send + Sync {
    fn transform_request(&self, req: &GatewayRequest) -> Result<BackendRequest, AppError>;

    fn execute(&self, req: &BackendRequest) -> impl std::future::Future<Output = Result<BackendResponse, AppError>> + Send;

    fn transform_response(&self, resp: &BackendResponse) -> Result<GatewayResponse, AppError>;

    fn name(&self) -> &str;
}
