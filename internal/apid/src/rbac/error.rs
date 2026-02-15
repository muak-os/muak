//! Error types for RBAC operations.

use thiserror::Error;

use sysconfig::Permission;

/// Errors that can occur during RBAC access checks.
#[derive(Debug, Error)]
pub enum RbacError {
    /// Client certificate is required but not provided.
    #[error("client certificate required")]
    Unauthenticated,

    /// Client certificate has been revoked.
    #[error("certificate has been revoked")]
    CertificateRevoked,

    /// Certificate fingerprint not found in authorized users.
    #[error("certificate not authorized")]
    UnknownCertificate,

    /// User lacks the required permission.
    #[error("permission denied: requires {required}")]
    PermissionDenied {
        /// The permission that was required.
        required: Permission,
    },

    /// Method not found in any known service.
    #[error("unknown method")]
    UnknownMethod,

    /// Insecure access attempted on an installed system.
    #[error("system is installed, insecure access is not allowed")]
    InsecureNotAllowed,
}

impl RbacError {
    /// Returns the appropriate gRPC status code for this error.
    #[must_use]
    pub const fn grpc_status_code(&self) -> u8 {
        match self {
            Self::Unauthenticated => 16,
            Self::CertificateRevoked
            | Self::UnknownCertificate
            | Self::PermissionDenied { .. }
            | Self::InsecureNotAllowed => 7,
            Self::UnknownMethod => 12,
        }
    }

    /// Returns the error message suitable for gRPC response.
    #[must_use]
    pub fn grpc_message(&self) -> String {
        self.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grpc_status_codes() {
        assert_eq!(RbacError::Unauthenticated.grpc_status_code(), 16);
        assert_eq!(RbacError::CertificateRevoked.grpc_status_code(), 7);
        assert_eq!(RbacError::UnknownCertificate.grpc_status_code(), 7);
        assert_eq!(
            RbacError::PermissionDenied {
                required: Permission::Admin
            }
            .grpc_status_code(),
            7
        );
        assert_eq!(RbacError::UnknownMethod.grpc_status_code(), 12);
        assert_eq!(RbacError::InsecureNotAllowed.grpc_status_code(), 7);
    }

    #[test]
    fn test_error_messages() {
        assert_eq!(
            RbacError::Unauthenticated.to_string(),
            "client certificate required"
        );
        assert_eq!(
            RbacError::PermissionDenied {
                required: Permission::VmCreate
            }
            .to_string(),
            "permission denied: requires vm:create"
        );
        assert_eq!(
            RbacError::InsecureNotAllowed.to_string(),
            "system is installed, insecure access is not allowed"
        );
    }
}
