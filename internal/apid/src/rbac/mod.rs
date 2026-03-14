//! Role-Based Access Control (RBAC) for the API gateway.

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
        MethodRequirement::RequiresPermission(required)
        | MethodRequirement::MaintenanceOrPermission(required) => {
            let fingerprint = client_fingerprint.ok_or(RbacError::Unauthenticated)?;

            let auth = config::auth();

            if auth.revoked.contains(&fingerprint.to_string()) {
                return Err(RbacError::CertificateRevoked);
            }

            let auth_user = auth
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
    fn unauthenticated_methods_allowed() {
        // ASSERT
        assert!(check_access("/muak.auth.v1.AuthService/SubmitCsr", None).is_ok());
        assert!(check_access("/muak.auth.v1.AuthService/GetCsrStatus", None).is_ok());
        assert!(check_access("/muak.auth.v1.AuthService/AckEnrollment", None).is_ok());
    }

    #[test]
    fn maintenance_methods_require_auth_outside_maintenance() {
        // ASSERT
        assert!(matches!(
            check_access("/muak.provision.v1.ProvisionService/Install", None),
            Err(RbacError::Unauthenticated)
        ));
        assert!(matches!(
            check_access("/muak.provision.v1.ProvisionService/ListDisks", None),
            Err(RbacError::Unauthenticated)
        ));
        assert!(matches!(
            check_access("/muak.log.v1.LogService/GetLogs", None),
            Err(RbacError::Unauthenticated)
        ));
        assert!(matches!(
            check_access("/muak.log.v1.LogService/FollowLogs", None),
            Err(RbacError::Unauthenticated)
        ));
    }

    #[test]
    fn maintenance_methods_are_maintenance_or_permission() {
        // ASSERT
        for path in MAINTENANCE_METHODS {
            assert!(
                matches!(
                    method_permission(path),
                    MethodRequirement::MaintenanceOrPermission(_)
                ),
                "Expected MaintenanceOrPermission for {}",
                path
            );
        }
    }

    #[test]
    fn unknown_method_denied() {
        // ARRANGE
        let path = "/muak.vm.v1.VmService/UnknownMethod";
        let fingerprint = "fp";

        // ACT
        let result = check_access(path, Some(fingerprint));

        // ASSERT
        assert!(matches!(result, Err(RbacError::UnknownMethod)));
    }

    #[test]
    fn unknown_service_denied() {
        // ARRANGE
        let path = "/unknown.Service/Method";
        let fingerprint = "fp";

        // ACT
        let result = check_access(path, Some(fingerprint));

        // ASSERT
        assert!(matches!(result, Err(RbacError::UnknownMethod)));
    }

    #[test]
    fn authenticated_without_cert_denied() {
        // ARRANGE
        let path = "/muak.vm.v1.VmService/CreateVm";

        // ACT
        let result = check_access(path, None);

        // ASSERT
        assert!(matches!(result, Err(RbacError::Unauthenticated)));
    }

    #[test]
    fn known_methods_not_empty() {
        // ARRANG
        let count = KNOWN_METHODS.len();

        // ASSERT
        assert!(!KNOWN_METHODS.is_empty());
        assert!(count > 0);
    }

    #[test]
    fn unauthenticated_methods_list() {
        // ASSERT
        assert!(UNAUTHENTICATED_METHODS.contains(&"/muak.auth.v1.AuthService/SubmitCsr"));
        assert!(UNAUTHENTICATED_METHODS.contains(&"/muak.auth.v1.AuthService/GetCsrStatus"));
        assert!(UNAUTHENTICATED_METHODS.contains(&"/muak.auth.v1.AuthService/AckEnrollment"));
        assert_eq!(UNAUTHENTICATED_METHODS.len(), 3);
    }

    #[test]
    fn maintenance_methods_list() {
        // ASSERT
        assert!(MAINTENANCE_METHODS.contains(&"/muak.provision.v1.ProvisionService/Install"));
        assert!(MAINTENANCE_METHODS.contains(&"/muak.provision.v1.ProvisionService/ListDisks"));
        assert!(MAINTENANCE_METHODS.contains(&"/muak.log.v1.LogService/GetLogs"));
        assert!(MAINTENANCE_METHODS.contains(&"/muak.log.v1.LogService/FollowLogs"));
        assert_eq!(MAINTENANCE_METHODS.len(), 4);
    }

    #[test]
    fn no_overlap_between_unauthenticated_and_maintenance() {
        // ASSERT
        for method in UNAUTHENTICATED_METHODS {
            assert!(
                !MAINTENANCE_METHODS.contains(method),
                "Method {} should not be in both unauthenticated and maintenance lists",
                method
            );
        }
    }
}
