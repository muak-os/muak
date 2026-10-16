//! gRPC service implementation for security queries.

use firmware::secureboot;
use tonic::{Request, Response, Status};

use super::proto::security::security_service_server::{SecurityService, SecurityServiceServer};
use super::proto::security::{GetSecurityStateRequest, GetSecurityStateResponse, SecureBootState};

/// Creates the `SecurityService` gRPC server.
pub fn service() -> SecurityServiceServer<ServiceImpl> {
    SecurityServiceServer::new(ServiceImpl)
}

/// Implementation of the `SecurityService` gRPC interface.
pub struct ServiceImpl;

#[tonic::async_trait]
impl SecurityService for ServiceImpl {
    async fn get_security_state(
        &self,
        _request: Request<GetSecurityStateRequest>,
    ) -> Result<Response<GetSecurityStateResponse>, Status> {
        let state = secureboot::system()
            .map_err(|e| Status::internal(format!("Failed to read Secure Boot state: {e}")))?;

        let installed_secureboot = config::system::host()
            .map_err(|e| Status::internal(e.to_string()))?
            .secureboot;
        let secure_boot_state = if state.enabled {
            SecureBootState::Enabled
        } else if installed_secureboot && state.pk_present {
            SecureBootState::Pending
        } else {
            SecureBootState::Disabled
        };

        Ok(Response::new(GetSecurityStateResponse {
            secure_boot: secure_boot_state.into(),
            setup_mode: state.setup_mode,
        }))
    }
}
