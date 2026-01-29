//! Authentication and authorization logic

use crate::config;

/// Result of authentication check
pub enum AuthResult {
    Allowed,
    /// Client certificate required but not provided
    Unauthenticated,
    Revoked,
    Unauthorized,
}

/// Checks if a request is authorized based on the client fingerprint and path.
pub fn check_auth(path: &str, client_fingerprint: Option<&str>) -> AuthResult {
    if config::UNAUTHENTICATED_METHODS.contains(&path) {
        return AuthResult::Allowed;
    }

    let fingerprint = match client_fingerprint {
        Some(fp) => fp,
        None => return AuthResult::Unauthenticated,
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
