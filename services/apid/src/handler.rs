//! gRPC request handling and routing

use std::sync::Arc;

use http_body_util::{Either, Empty};
use hyper::body::{Bytes, Incoming};
use hyper::{Request, Response};

use crate::constants;
use crate::proxy::{self, BackendPool};
use crate::rbac;

/// Body type for handler responses - either an error (Empty) or proxied stream (Incoming)
pub type Body = Either<Empty<Bytes>, Incoming>;

/// Handles an incoming gRPC request
pub async fn handle_request(
    pool: &BackendPool,
    req: Request<Incoming>,
    client_fingerprint: Option<Arc<str>>,
    maintenance_mode: bool,
) -> Result<Response<Body>, hyper::Error> {
    let path = req.uri().path();
    let requirement = rbac::method_permission(path);

    let skip_rbac = match &requirement {
        rbac::MethodRequirement::Unauthenticated => true,
        rbac::MethodRequirement::MaintenanceOrPermission(_) if maintenance_mode => true,
        _ => false,
    };

    if !skip_rbac
        && let Err(e) = rbac::check_access(path, client_fingerprint.as_deref()) {
            kmsg::warn!("Access denied for {}: {}", path, e);
            return Ok(grpc_error(e.grpc_status_code(), &e.grpc_message()));
        }

    if !maintenance_mode
        && client_fingerprint.is_none()
        && !matches!(requirement, rbac::MethodRequirement::Unauthenticated)
    {
        let e = rbac::RbacError::InsecureNotAllowed;
        kmsg::warn!("Insecure access blocked for {}: {}", path, e);
        return Ok(grpc_error(e.grpc_status_code(), &e.grpc_message()));
    }

    let socket_path = match route_request(path).await {
        Some(socket) => socket,
        None => {
            kmsg::warn!("Unknown service path: {}", path);
            return Ok(grpc_error(12, "Unknown service"));
        }
    };

    match proxy::proxy_to_backend(pool, req, socket_path).await {
        Ok(response) => Ok(response.map(Either::Right)),
        Err(e) => {
            kmsg::error!("Proxy error to {}: {}", socket_path, e);
            Ok(grpc_error(14, &format!("Backend unavailable: {}", e)))
        }
    }
}

/// Routes a request path to the appropriate backend socket.
pub async fn route_request(path: &str) -> Option<&'static str> {
    if path.starts_with(constants::VM_SERVICE_PREFIX) {
        let socket_exists = tokio::fs::try_exists(constants::VMD_SOCKET)
            .await
            .unwrap_or(false);
        if !socket_exists {
            kmsg::warn!("VM service not available in maintenance mode");
            return None;
        }
        Some(constants::VMD_SOCKET)
    } else if path.starts_with(constants::PROCESS_SERVICE_PREFIX)
        || path.starts_with(constants::LOG_SERVICE_PREFIX)
    {
        Some(constants::GRANOLA_SOCKET)
    } else if path.starts_with(constants::PROVISION_SERVICE_PREFIX)
        || path.starts_with(constants::AUTH_SERVICE_PREFIX)
        || path.starts_with(constants::SECURITY_SERVICE_PREFIX)
        || path.starts_with(constants::VERSION_SERVICE_PREFIX)
    {
        Some(constants::PROVISIOND_SOCKET)
    } else {
        None
    }
}

/// Creates a gRPC error response with the given status code and message.
pub fn grpc_error(status: u8, message: &str) -> Response<Body> {
    Response::builder()
        .status(200)
        .header("content-type", "application/grpc")
        .header("grpc-status", status.to_string())
        .header("grpc-message", message)
        .body(Either::Left(Empty::new()))
        .expect("building response should not fail")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn route_request_process_service() {
        // ARRANGE
        let path = "/muak.process.v1.ProcessService/List";

        // ACT
        let socket = route_request(path).await;

        // ASSERT
        assert_eq!(socket, Some(constants::GRANOLA_SOCKET));
    }

    #[tokio::test]
    async fn route_request_log_service() {
        // ARRANGE
        let path = "/muak.log.v1.LogService/GetLogs";

        // ACT
        let socket = route_request(path).await;

        // ASSERT
        assert_eq!(socket, Some(constants::GRANOLA_SOCKET));
    }

    #[tokio::test]
    async fn route_request_log_service_follow() {
        // ARRANGE
        let path = "/muak.log.v1.LogService/FollowLogs";

        // ACT
        let socket = route_request(path).await;

        // ASSERT
        assert_eq!(socket, Some(constants::GRANOLA_SOCKET));
    }

    #[tokio::test]
    async fn route_request_provision_service() {
        // ARRANGE
        let path = "/muak.provision.v1.ProvisionService/Install";

        // ACT
        let socket = route_request(path).await;

        // ASSERT
        assert_eq!(socket, Some(constants::PROVISIOND_SOCKET));
    }

    #[tokio::test]
    async fn route_request_auth_service() {
        // ARRANGE
        let path = "/muak.auth.v1.AuthService/SubmitCsr";

        // ACT
        let socket = route_request(path).await;

        // ASSERT
        assert_eq!(socket, Some(constants::PROVISIOND_SOCKET));
    }

    #[tokio::test]
    async fn route_request_security_service() {
        // ARRANGE
        let path = "/muak.security.v1.SecurityService/GetSecurityState";

        // ACT
        let socket = route_request(path).await;

        // ASSERT
        assert_eq!(socket, Some(constants::PROVISIOND_SOCKET));
    }

    #[tokio::test]
    async fn route_request_version_service() {
        // ARRANGE
        let path = "/muak.version.v1.VersionService/GetVersion";

        // ACT
        let socket = route_request(path).await;

        // ASSERT
        assert_eq!(socket, Some(constants::PROVISIOND_SOCKET));
    }

    #[tokio::test]
    async fn route_request_unknown_service() {
        // ARRANGE
        let path = "/unknown.Service/Method";

        // ACT
        let socket = route_request(path).await;

        // ASSERT
        assert_eq!(socket, None);
    }

    #[tokio::test]
    async fn route_request_empty_path() {
        // ARRANGE
        let path = "";

        // ACT
        let socket = route_request(path).await;

        // ASSERT
        assert_eq!(socket, None);
    }

    #[tokio::test]
    async fn route_request_root_path() {
        // ARRANGE
        let path = "/";

        // ACT
        let socket = route_request(path).await;

        // ASSERT
        assert_eq!(socket, None);
    }

    #[tokio::test]
    async fn route_request_partial_match_not_enough() {
        // ARRANGE
        let socket_path = "/muak.process";

        // ACT
        let socket = route_request(socket_path).await;

        // ASSERT
        assert_eq!(socket, None);
    }

    #[tokio::test]
    async fn route_request_vm_service_without_socket() {
        // ARRANGE
        let path = "/muak.vm.v1.VmService/CreateVm";

        // ACT
        let socket = route_request(path).await;

        // ASSERT
        assert_eq!(socket, None);
    }

    #[test]
    fn grpc_error_response_status_is_200() {
        // ARRANGE
        let status_code = 7;
        let message = "test message";

        // ACT
        let response = grpc_error(status_code, message);

        // ASSERT
        assert_eq!(response.status().as_u16(), 200);
    }

    #[test]
    fn grpc_error_content_type() {
        // ARRANGE
        let status_code = 7;
        let message = "test message";

        // ACT
        let response = grpc_error(status_code, message);
        let content_type = response.headers().get("content-type").unwrap();

        // ASSERT
        assert_eq!(content_type, "application/grpc");
    }

    #[test]
    fn grpc_error_status_header() {
        // ARRANGE
        let status_code = 7;
        let message = "permission denied";

        // ACT
        let response = grpc_error(status_code, message);
        let grpc_status = response.headers().get("grpc-status").unwrap();

        // ASSERT
        assert_eq!(grpc_status, "7");
    }

    #[test]
    fn grpc_error_message_header() {
        // ARRANGE
        let status_code = 12;
        let message = "method not found";

        // ACT
        let response = grpc_error(status_code, message);
        let grpc_message = response.headers().get("grpc-message").unwrap();

        // ASSERT
        assert_eq!(grpc_message, "method not found");
    }

    #[test]
    fn grpc_error_unauthenticated() {
        // ARRANGE
        let status_code = 16;
        let message = "client certificate required";

        // ACT
        let response = grpc_error(status_code, message);
        let grpc_status = response.headers().get("grpc-status").unwrap();

        // ASSERT
        assert_eq!(grpc_status, "16");
    }

    #[test]
    fn grpc_error_unavailable() {
        // ARRANGE
        let status_code = 14;
        let message = "Backend unavailable: connection refused";

        // ACT
        let response = grpc_error(status_code, message);
        let grpc_status = response.headers().get("grpc-status").unwrap();
        let grpc_message = response.headers().get("grpc-message").unwrap();

        // ASSERT
        assert_eq!(grpc_status, "14");
        assert!(
            grpc_message
                .to_str()
                .unwrap()
                .contains("Backend unavailable")
        );
    }

    #[test]
    fn grpc_error_empty_message() {
        // ARRANGE
        let status_code = 0;
        let message = "";

        // ACT
        let response = grpc_error(status_code, message);
        let grpc_message = response.headers().get("grpc-message").unwrap();

        // ASSERT
        assert_eq!(grpc_message, "");
    }

    #[test]
    fn service_prefixes_end_with_slash() {
        // ARRANGE & ACT & ASSERT
        assert!(constants::VM_SERVICE_PREFIX.ends_with('/'));
        assert!(constants::PROCESS_SERVICE_PREFIX.ends_with('/'));
        assert!(constants::PROVISION_SERVICE_PREFIX.ends_with('/'));
        assert!(constants::AUTH_SERVICE_PREFIX.ends_with('/'));
        assert!(constants::SECURITY_SERVICE_PREFIX.ends_with('/'));
        assert!(constants::LOG_SERVICE_PREFIX.ends_with('/'));
        assert!(constants::VERSION_SERVICE_PREFIX.ends_with('/'));
    }

    #[test]
    fn service_prefixes_start_with_slash() {
        // ARRANGE & ACT & ASSERT
        assert!(constants::VM_SERVICE_PREFIX.starts_with('/'));
        assert!(constants::PROCESS_SERVICE_PREFIX.starts_with('/'));
        assert!(constants::PROVISION_SERVICE_PREFIX.starts_with('/'));
        assert!(constants::AUTH_SERVICE_PREFIX.starts_with('/'));
        assert!(constants::SECURITY_SERVICE_PREFIX.starts_with('/'));
        assert!(constants::LOG_SERVICE_PREFIX.starts_with('/'));
        assert!(constants::VERSION_SERVICE_PREFIX.starts_with('/'));
    }
}
