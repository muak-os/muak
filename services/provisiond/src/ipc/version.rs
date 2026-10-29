//! `VersionService` gRPC implementation.

use tonic::{Request, Response, Status};

use super::proto::version::version_service_server::{VersionService, VersionServiceServer};
use super::proto::version::{GetVersionRequest, GetVersionResponse};

/// Implementation of the `VersionService` gRPC interface.
pub struct ServiceImpl;

#[tonic::async_trait]
impl VersionService for ServiceImpl {
    async fn get_version(
        &self,
        _request: Request<GetVersionRequest>,
    ) -> Result<Response<GetVersionResponse>, Status> {
        let version = config::system::host()
            .map_err(|error| {
                Status::failed_precondition(format!("configuration unavailable: {error}"))
            })?
            .version
            .clone();

        Ok(Response::new(GetVersionResponse { version }))
    }
}

/// Creates the `VersionService` gRPC server.
pub fn service() -> VersionServiceServer<ServiceImpl> {
    VersionServiceServer::new(ServiceImpl)
}
