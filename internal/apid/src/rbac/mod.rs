//! Role-Based Access Control (RBAC) for the API gateway.
//!
//! This module provides declarative permission rules and access checking
//! for gRPC methods. All methods are deny-by-default unless explicitly
//! listed in the rules.
//!
//! # Architecture
//!
//! RBAC rules are generated at build time from proto file annotations.
//! Each RPC method in `api/*.proto` must have an `// @rbac:` comment
//! specifying its permission requirements.
//!
//! - `error`: Error types for access control failures
//! - `user`: Authenticated user wrapper with permission helpers
//! - Generated code provides `MethodRequirement` and `method_permission()`
//!
//! # Example
//!
//! ```ignore
//! use crate::rbac::check_access;
//!
//! let result = check_access(path, client_fingerprint.as_deref());
//! match result {
//!     Ok(()) => { /* proceed with request */ }
//!     Err(e) => return grpc_error(e.grpc_status_code(), &e.grpc_message()),
//! }
//! ```

mod error;
mod user;

// Include the generated RBAC rules from build.rs
include!(concat!(env!("OUT_DIR"), "/rbac_rules.rs"));

pub use error::RbacError;
use user::AuthenticatedUser;

/// Checks if a request is authorized based on the path and client fingerprint.
pub fn check_access(path: &str, client_fingerprint: Option<&str>) -> Result<(), RbacError> {
    match method_permission(path) {
        MethodRequirement::Unauthenticated => Ok(()),
        MethodRequirement::Unknown => Err(RbacError::UnknownMethod),
        MethodRequirement::RequiresPermission(required) => {
            let fingerprint = client_fingerprint.ok_or(RbacError::Unauthenticated)?;

            if sysconfig::auth().revoked.contains(&fingerprint.to_string()) {
                return Err(RbacError::CertificateRevoked);
            }

            let auth_user = sysconfig::auth()
                .users
                .iter()
                .find(|u| u.fingerprint == fingerprint)
                .ok_or(RbacError::UnknownCertificate)?;

            let user = AuthenticatedUser::from(auth_user);

            if !user.has_permission(required) {
                return Err(RbacError::PermissionDenied { required });
            }

            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unauthenticated_methods_allowed() {
        assert!(check_access("/muak.auth.v1.AuthService/SubmitCsr", None).is_ok());
        assert!(check_access("/muak.provision.v1.ProvisionService/ListDisks", None).is_ok());
        assert!(check_access("/muak.log.v1.LogService/GetLogs", None).is_ok());
        assert!(check_access("/muak.log.v1.LogService/FollowLogs", None).is_ok());
    }

    #[test]
    fn test_unknown_method_denied() {
        let result = check_access("/muak.vm.v1.VmService/UnknownMethod", Some("fp"));
        assert!(matches!(result, Err(RbacError::UnknownMethod)));
    }

    #[test]
    fn test_unknown_service_denied() {
        let result = check_access("/unknown.Service/Method", Some("fp"));
        assert!(matches!(result, Err(RbacError::UnknownMethod)));
    }

    #[test]
    fn test_authenticated_without_cert_denied() {
        let result = check_access("/muak.vm.v1.VmService/CreateVm", None);
        assert!(matches!(result, Err(RbacError::Unauthenticated)));
    }

    #[test]
    fn test_known_methods_not_empty() {
        assert!(!KNOWN_METHODS.is_empty());
    }

    #[test]
    fn test_unauthenticated_methods_list() {
        assert!(UNAUTHENTICATED_METHODS.contains(&"/muak.auth.v1.AuthService/SubmitCsr"));
        assert!(UNAUTHENTICATED_METHODS.contains(&"/muak.auth.v1.AuthService/GetCsrStatus"));
        assert!(UNAUTHENTICATED_METHODS.contains(&"/muak.auth.v1.AuthService/AckEnrollment"));
        assert!(UNAUTHENTICATED_METHODS.contains(&"/muak.provision.v1.ProvisionService/Install"));
        assert!(UNAUTHENTICATED_METHODS.contains(&"/muak.provision.v1.ProvisionService/ListDisks"));
        assert!(UNAUTHENTICATED_METHODS.contains(&"/muak.log.v1.LogService/GetLogs"));
        assert!(UNAUTHENTICATED_METHODS.contains(&"/muak.log.v1.LogService/FollowLogs"));
        assert_eq!(UNAUTHENTICATED_METHODS.len(), 7);
    }
}
