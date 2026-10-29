//! gRPC service implementation for security queries.

use firmware::secureboot;
use tonic::{Request, Response, Status};

use super::proto::security::security_service_server::{SecurityService, SecurityServiceServer};
use super::proto::security::{
    GetSecurityStateRequest, GetSecurityStateResponse, SecureBootDeviation, SecureBootState,
};

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
        let config_enabled = config::system::host()
            .map_err(|e| Status::internal(e.to_string()))?
            .secureboot;

        Ok(Response::new(response(state, config_enabled)))
    }
}

/// Creates the `SecurityService` gRPC server.
pub fn service() -> SecurityServiceServer<ServiceImpl> {
    SecurityServiceServer::new(ServiceImpl)
}

fn response(state: secureboot::State, config_enabled: bool) -> GetSecurityStateResponse {
    GetSecurityStateResponse {
        secure_boot: status_value(state).into(),
        setup_mode: state.setup_mode,
        config_enabled,
        deviation: deviation_value(config_enabled, state).into(),
    }
}

fn status_value(state: secureboot::State) -> SecureBootState {
    match secureboot::status(&state) {
        secureboot::Status::Enabled => SecureBootState::Enabled,
        secureboot::Status::Pending => SecureBootState::Pending,
        secureboot::Status::Disabled => SecureBootState::Disabled,
    }
}

fn deviation_value(config_enabled: bool, state: secureboot::State) -> SecureBootDeviation {
    match secureboot::mismatch(config_enabled, &state) {
        None => SecureBootDeviation::None,
        Some(secureboot::Mismatch::NotEnrolled) => SecureBootDeviation::ConfigEnabledNoKeys,
        Some(secureboot::Mismatch::KeysRemain) => SecureBootDeviation::KeysRemain,
        Some(secureboot::Mismatch::EnforcingWhileDisabled) => {
            SecureBootDeviation::EnforcingWhileDisabled
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_reports_firmware_posture_and_intent() {
        // ARRANGE
        let state = secureboot::State {
            enabled: false,
            setup_mode: true,
            pk_present: false,
        };

        // ACT
        let response = response(state, true);

        // ASSERT
        assert_eq!(response.secure_boot, i32::from(SecureBootState::Disabled));
        assert!(response.setup_mode);
        assert!(response.config_enabled);
        assert_eq!(
            response.deviation,
            i32::from(SecureBootDeviation::ConfigEnabledNoKeys)
        );
    }

    #[test]
    fn response_has_no_deviation_when_intent_matches() {
        // ARRANGE
        let state = secureboot::State {
            enabled: true,
            setup_mode: false,
            pk_present: true,
        };

        // ACT
        let response = response(state, true);

        // ASSERT
        assert_eq!(response.secure_boot, i32::from(SecureBootState::Enabled));
        assert_eq!(response.deviation, i32::from(SecureBootDeviation::None));
    }

    #[test]
    fn response_reports_pending_and_leftover_keys() {
        // ARRANGE
        let state = secureboot::State {
            enabled: false,
            setup_mode: false,
            pk_present: true,
        };

        // ACT
        let response = response(state, false);

        // ASSERT
        assert_eq!(response.secure_boot, i32::from(SecureBootState::Pending));
        assert_eq!(
            response.deviation,
            i32::from(SecureBootDeviation::KeysRemain)
        );
    }
}
