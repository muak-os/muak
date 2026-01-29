//! Authentication and authorization logic

use crate::config;

/// Result of authentication check
pub enum AuthResult {
    Allowed,
    /// Server is in maintenance mode (not installed)
    MaintenanceMode,
    /// Client certificate required but not provided
    Unauthenticated,
    Revoked,
    Unauthorized,
}

/// Returns true if the server has been installed (PKI is present).
pub fn is_installed() -> bool {
    std::path::Path::new(config::CA_CERT_PATH).exists()
}

/// Checks if a request is authorized based on the client fingerprint and path.
pub fn check_auth(path: &str, client_fingerprint: Option<&str>) -> AuthResult {
    if config::UNAUTHENTICATED_METHODS.contains(&path) {
        return AuthResult::Allowed;
    }

    let fingerprint = match client_fingerprint {
        Some(fp) => fp,
        None => {
            // No client cert - check if we're in maintenance mode or installed
            return if is_installed() {
                AuthResult::Unauthenticated
            } else {
                AuthResult::MaintenanceMode
            };
        }
    };

    if sysconfig::auth().revoked.contains(&fingerprint.to_string()) {
        return AuthResult::Revoked;
    }

    let user = sysconfig::auth()
        .users
        .iter()
        .find(|u| u.fingerprint == fingerprint);

    if user.is_none() {
        return AuthResult::Unauthorized;
    }

    // TODO: Check specific permissions based on path
    // For now, any authorized user can access protected endpoints

    AuthResult::Allowed
}
