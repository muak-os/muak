//! gRPC request handling and routing

use http_body_util::{Either, Empty};
use hyper::body::{Bytes, Incoming};
use hyper::{Request, Response};

use crate::config;
use crate::proxy;
use crate::rbac;

/// Body type for handler responses - either an error (Empty) or proxied stream (Incoming)
pub type Body = Either<Empty<Bytes>, Incoming>;

/// Handles an incoming gRPC request
pub async fn handle_request(
    req: Request<Incoming>,
    client_fingerprint: Option<String>,
) -> Result<Response<Body>, hyper::Error> {
    let path = req.uri().path();

    if let Err(e) = rbac::check_access(path, client_fingerprint.as_deref()) {
        kmsg::warn!("Access denied for {}: {}", path, e);
        return Ok(grpc_error(e.grpc_status_code(), &e.grpc_message()));
    }

    let socket_path = match route_request(path) {
        Some(socket) => socket,
        None => {
            kmsg::warn!("Unknown service path: {}", path);
            return Ok(grpc_error(12, "Unknown service"));
        }
    };

    match proxy::proxy_to_backend(req, socket_path).await {
        Ok(response) => Ok(response.map(Either::Right)),
        Err(e) => {
            kmsg::error!("Proxy error to {}: {}", socket_path, e);
            Ok(grpc_error(14, &format!("Backend unavailable: {}", e)))
        }
    }
}

/// Routes a request path to the appropriate backend socket.
pub fn route_request(path: &str) -> Option<&'static str> {
    if path.starts_with(config::VM_SERVICE_PREFIX) {
        if !std::path::Path::new(config::VMD_SOCKET).exists() {
            kmsg::warn!("VM service not available in maintenance mode");
            return None;
        }
        Some(config::VMD_SOCKET)
    } else if path.starts_with(config::PROCESS_SERVICE_PREFIX) {
        Some(config::GRANOLA_SOCKET)
    } else if path.starts_with(config::PROVISION_SERVICE_PREFIX)
        || path.starts_with(config::AUTH_SERVICE_PREFIX)
        || path.starts_with(config::SECURITY_SERVICE_PREFIX)
    {
        Some(config::PROVISIOND_SOCKET)
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

    #[test]
    fn test_route_request_process_service() {
        let path = "/muak.process.v1.ProcessService/List";
        let socket = route_request(path);
        assert_eq!(socket, Some(config::GRANOLA_SOCKET));
    }

    #[test]
    fn test_route_request_provision_service() {
        let path = "/muak.provision.v1.ProvisionService/Install";
        let socket = route_request(path);
        assert_eq!(socket, Some(config::PROVISIOND_SOCKET));
    }

    #[test]
    fn test_route_request_auth_service() {
        let path = "/muak.auth.v1.AuthService/SubmitCsr";
        let socket = route_request(path);
        assert_eq!(socket, Some(config::PROVISIOND_SOCKET));
    }

    #[test]
    fn test_route_request_security_service() {
        let path = "/muak.security.v1.SecurityService/GetSecurityState";
        let socket = route_request(path);
        assert_eq!(socket, Some(config::PROVISIOND_SOCKET));
    }

    #[test]
    fn test_route_request_unknown_service() {
        let path = "/unknown.Service/Method";
        let socket = route_request(path);
        assert_eq!(socket, None);
    }

    #[test]
    fn test_route_request_empty_path() {
        let socket = route_request("");
        assert_eq!(socket, None);
    }

    #[test]
    fn test_route_request_root_path() {
        let socket = route_request("/");
        assert_eq!(socket, None);
    }

    #[test]
    fn test_route_request_partial_match_not_enough() {
        let socket = route_request("/muak.process");
        assert_eq!(socket, None);
    }

    #[test]
    fn test_route_request_vm_service_without_socket() {
        let path = "/muak.vm.v1.VmService/CreateVm";
        let socket = route_request(path);
        assert_eq!(socket, None);
    }

    #[test]
    fn test_grpc_error_response_status_is_200() {
        let response = grpc_error(7, "test message");
        assert_eq!(response.status().as_u16(), 200);
    }

    #[test]
    fn test_grpc_error_content_type() {
        let response = grpc_error(7, "test message");
        let content_type = response.headers().get("content-type").unwrap();
        assert_eq!(content_type, "application/grpc");
    }

    #[test]
    fn test_grpc_error_status_header() {
        let response = grpc_error(7, "permission denied");
        let grpc_status = response.headers().get("grpc-status").unwrap();
        assert_eq!(grpc_status, "7");
    }

    #[test]
    fn test_grpc_error_message_header() {
        let response = grpc_error(12, "method not found");
        let grpc_message = response.headers().get("grpc-message").unwrap();
        assert_eq!(grpc_message, "method not found");
    }

    #[test]
    fn test_grpc_error_unauthenticated() {
        let response = grpc_error(16, "client certificate required");
        let grpc_status = response.headers().get("grpc-status").unwrap();
        assert_eq!(grpc_status, "16");
    }

    #[test]
    fn test_grpc_error_unavailable() {
        let response = grpc_error(14, "Backend unavailable: connection refused");
        let grpc_status = response.headers().get("grpc-status").unwrap();
        let grpc_message = response.headers().get("grpc-message").unwrap();
        assert_eq!(grpc_status, "14");
        assert!(
            grpc_message
                .to_str()
                .unwrap()
                .contains("Backend unavailable")
        );
    }

    #[test]
    fn test_grpc_error_empty_message() {
        let response = grpc_error(0, "");
        let grpc_message = response.headers().get("grpc-message").unwrap();
        assert_eq!(grpc_message, "");
    }

    #[test]
    fn test_service_prefixes_end_with_slash() {
        assert!(config::VM_SERVICE_PREFIX.ends_with('/'));
        assert!(config::PROCESS_SERVICE_PREFIX.ends_with('/'));
        assert!(config::PROVISION_SERVICE_PREFIX.ends_with('/'));
        assert!(config::AUTH_SERVICE_PREFIX.ends_with('/'));
        assert!(config::SECURITY_SERVICE_PREFIX.ends_with('/'));
    }

    #[test]
    fn test_service_prefixes_start_with_slash() {
        assert!(config::VM_SERVICE_PREFIX.starts_with('/'));
        assert!(config::PROCESS_SERVICE_PREFIX.starts_with('/'));
        assert!(config::PROVISION_SERVICE_PREFIX.starts_with('/'));
        assert!(config::AUTH_SERVICE_PREFIX.starts_with('/'));
        assert!(config::SECURITY_SERVICE_PREFIX.starts_with('/'));
    }
}
