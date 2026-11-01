//! gRPC service implementation for security queries.

use std::path::Path;

use firmware::secureboot;
use tonic::{Request, Response, Status};

use super::proto::security::security_service_server::{SecurityService, SecurityServiceServer};
use super::proto::security::{
    GetSecurityStateRequest, GetSecurityStateResponse, SecureBootDeviation, SecureBootState,
};
use crate::secrets::SECRETS_DIR;

/// Implementation of the `SecurityService` gRPC interface.
pub struct ServiceImpl;

#[tonic::async_trait]
impl SecurityService for ServiceImpl {
    async fn get_security_state(
        &self,
        _request: Request<GetSecurityStateRequest>,
    ) -> Result<Response<GetSecurityStateResponse>, Status> {
        let store = firmware::system();
        let state = secureboot::state(&store)
            .map_err(|e| Status::internal(format!("Failed to read Secure Boot state: {e}")))?;
        let config_enabled = config::system::host()
            .map_err(|e| Status::internal(e.to_string()))?
            .secureboot;
        let ours = platform_key_matches(&store);

        Ok(Response::new(response(state, config_enabled, ours)))
    }
}

/// Creates the `SecurityService` gRPC server.
pub fn service() -> SecurityServiceServer<ServiceImpl> {
    SecurityServiceServer::new(ServiceImpl)
}

fn platform_key_matches<S: firmware::nvram::VariableStore>(store: &S) -> bool {
    let Ok(certificate) = std::fs::read(Path::new(SECRETS_DIR).join("secureboot/pk.der")) else {
        return false;
    };

    secureboot::platform_key_matches(store, &certificate).unwrap_or(false)
}

fn response(
    state: secureboot::State,
    config_enabled: bool,
    ours: bool,
) -> GetSecurityStateResponse {
    GetSecurityStateResponse {
        secure_boot: status_value(state, ours).into(),
        setup_mode: state.setup_mode,
        config_enabled,
        deviation: deviation_value(config_enabled, state, ours).into(),
    }
}

fn status_value(state: secureboot::State, ours: bool) -> SecureBootState {
    match secureboot::status(&state, ours) {
        secureboot::Status::Enabled => SecureBootState::Enabled,
        secureboot::Status::Pending => SecureBootState::Pending,
        secureboot::Status::Disabled => SecureBootState::Disabled,
    }
}

fn deviation_value(
    config_enabled: bool,
    state: secureboot::State,
    ours: bool,
) -> SecureBootDeviation {
    match secureboot::mismatch(config_enabled, &state, ours) {
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
        let response = response(state, true, false);

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
        let response = response(state, true, true);

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
        let response = response(state, false, true);

        // ASSERT
        assert_eq!(response.secure_boot, i32::from(SecureBootState::Pending));
        assert_eq!(
            response.deviation,
            i32::from(SecureBootDeviation::KeysRemain)
        );
    }

    #[test]
    fn response_ignores_foreign_pending_keys() {
        // ARRANGE
        let state = secureboot::State {
            enabled: false,
            setup_mode: false,
            pk_present: true,
        };

        // ACT
        let response = response(state, false, false);

        // ASSERT
        assert_eq!(response.secure_boot, i32::from(SecureBootState::Disabled));
        assert_eq!(response.deviation, i32::from(SecureBootDeviation::None));
    }
}
