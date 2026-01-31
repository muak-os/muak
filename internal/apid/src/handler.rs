//! gRPC request handling and routing

use http_body_util::{Either, Empty};
use hyper::body::{Bytes, Incoming};
use hyper::{Request, Response};

use crate::auth::{self, AuthResult};
use crate::config;
use crate::proxy;

/// Body type for handler responses - either an error (Empty) or proxied stream (Incoming)
pub type Body = Either<Empty<Bytes>, Incoming>;

/// Handles an incoming gRPC request
pub async fn handle_request(
    req: Request<Incoming>,
    client_fingerprint: Option<String>,
) -> Result<Response<Body>, hyper::Error> {
    let path = req.uri().path();

    match auth::check_auth(path, client_fingerprint.as_deref()) {
        AuthResult::Allowed => {}
        AuthResult::Unauthenticated => {
            kmsg::warn!("Unauthenticated request to protected endpoint: {}", path);
            return Ok(grpc_error(16, "Client certificate required"));
        }
        AuthResult::Revoked => {
            kmsg::warn!(
                "Revoked certificate attempted access: {}",
                client_fingerprint.as_deref().unwrap_or("unknown")
            );
            return Ok(grpc_error(7, "Certificate has been revoked"));
        }
        AuthResult::Unauthorized => {
            kmsg::warn!(
                "Unknown fingerprint attempted access: {}",
                client_fingerprint.as_deref().unwrap_or("unknown")
            );
            return Ok(grpc_error(7, "Certificate not authorized"));
        }
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

fn route_request(path: &str) -> Option<&'static str> {
    if path.starts_with(config::VM_SERVICE_PREFIX) {
        if !std::path::Path::new(config::VMD_SOCKET).exists() {
            kmsg::warn!("VM service not available in maintenance mode");
            return None;
        }
        Some(config::VMD_SOCKET)
    } else if path.starts_with(config::PROCESS_SERVICE_PREFIX)
        || path.starts_with(config::PROVISION_SERVICE_PREFIX)
        || path.starts_with(config::AUTH_SERVICE_PREFIX)
    {
        Some(config::GRANOLA_SOCKET)
    } else {
        None
    }
}

/// Creates a gRPC error response
fn grpc_error(status: u8, message: &str) -> Response<Body> {
    Response::builder()
        .status(200)
        .header("content-type", "application/grpc")
        .header("grpc-status", status.to_string())
        .header("grpc-message", message)
        .body(Either::Left(Empty::new()))
        .expect("building response should not fail")
}
