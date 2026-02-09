//! gRPC service implementation for security queries.

use tonic::{Request, Response, Status};

use super::proto::security::security_service_server::{SecurityService, SecurityServiceServer};
use super::proto::security::{GetSecurityStateRequest, GetSecurityStateResponse};

/// Creates the SecurityService gRPC server.
pub fn service() -> SecurityServiceServer<SecurityServiceImpl> {
    SecurityServiceServer::new(SecurityServiceImpl)
}

/// Implementation of the SecurityService gRPC interface.
pub struct SecurityServiceImpl;

#[tonic::async_trait]
impl SecurityService for SecurityServiceImpl {
    async fn get_security_state(
        &self,
        _request: Request<GetSecurityStateRequest>,
    ) -> Result<Response<GetSecurityStateResponse>, Status> {
        let secure_boot_enabled = sbolt::efi::get_secure_boot()
            .map_err(|e| Status::internal(format!("Failed to read Secure Boot state: {}", e)))?;

        Ok(Response::new(GetSecurityStateResponse {
            secure_boot_enabled,
        }))
    }
}
