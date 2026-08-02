//! gRPC service implementation for security queries.

use sbolt::efi;
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
        let enabled = efi::secure_boot()
            .map_err(|e| Status::internal(format!("Failed to read Secure Boot state: {e}")))?;

        let state = if enabled {
            SecureBootState::Enabled
        } else if config::host().secureboot
            && efi::pk()
                .map_err(|e| Status::internal(format!("Failed to read PK from firmware: {e}")))?
                .is_some()
        {
            SecureBootState::Pending
        } else {
            SecureBootState::Disabled
        };

        let setup_mode = efi::setup_mode()
            .map_err(|e| Status::internal(format!("Failed to read Setup Mode state: {e}")))?;

        Ok(Response::new(GetSecurityStateResponse {
            secure_boot: state.into(),
            setup_mode,
        }))
    }
}
