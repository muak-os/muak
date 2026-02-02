//! `apid` - API Gateway Daemon for Muak
//!
//! This library provides the core functionality for the Muak API gateway,
//! including mTLS authentication, RBAC enforcement, and HTTP/2 reverse
//! proxying to backend services.
//!
//! # Architecture
//!
//! The API gateway operates as follows:
//! 1. Accepts TLS connections (with optional client certificates)
//! 2. Enforces RBAC based on gRPC method paths and client certificate fingerprints
//! 3. Proxies authorized requests to backend services via UNIX sockets
//!
//! # Modules
//!
//! - [`config`]: Static configuration constants (paths, service prefixes)
//! - [`tls`]: TLS configuration (persistent and ephemeral certificates)
//! - [`server`]: HTTP/2 connection serving
//! - [`handler`]: Request routing and RBAC enforcement
//! - [`proxy`]: Backend communication via UNIX sockets
//! - [`rbac`]: Role-Based Access Control logic

pub mod config;
pub mod handler;
pub mod proxy;
pub mod rbac;
pub mod server;
pub mod tls;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

/// Arguments parsed from command line
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub listen_addr: String,
    pub maintenance_mode: bool,
}

impl Args {
    /// Creates new command line arguments struct
    pub fn new(listen_addr: String, maintenance_mode: bool) -> Self {
        Self {
            listen_addr,
            maintenance_mode,
        }
    }
}

/// Parses command line arguments into an Args struct.
pub fn parse_args(args: &[String]) -> Args {
    let default_listen = format!("0.0.0.0:{}", sysconfig::system().port);

    let listen_addr = args
        .iter()
        .position(|a| a == "--listen")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or(default_listen);

    let maintenance_mode = args.iter().any(|a| a == "--maintenance");

    Args {
        listen_addr,
        maintenance_mode,
    }
}

/// Sets up TLS configuration based on the mode.
pub fn setup_tls(maintenance_mode: bool) -> Result<TlsAcceptor> {
    if maintenance_mode {
        tls::generate_ephemeral_tls_config()
    } else {
        tls::load_tls_config()
    }
}

/// Runs the main loop for incoming connections.
pub async fn run(listener: &TcpListener, tls_acceptor: &TlsAcceptor, shutdown: &Arc<AtomicBool>) {
    while !shutdown.load(Ordering::SeqCst) {
        let accept_future = listener.accept();
        let timeout_result =
            tokio::time::timeout(std::time::Duration::from_millis(100), accept_future).await;

        let (stream, peer_addr) = match timeout_result {
            Ok(Ok(conn)) => conn,
            Ok(Err(e)) => {
                kmsg::warn!("Accept error: {}", e);
                continue;
            }
            Err(_) => continue,
        };

        let acceptor = tls_acceptor.clone();
        tokio::spawn(handle_tls_connection(acceptor, stream, peer_addr));
    }
}

/// Handles the TLS handshake and connection for a single client.
async fn handle_tls_connection(
    acceptor: TlsAcceptor,
    stream: tokio::net::TcpStream,
    peer_addr: SocketAddr,
) {
    match acceptor.accept(stream).await {
        Ok(tls_stream) => {
            let client_cert = tls_stream
                .get_ref()
                .1
                .peer_certificates()
                .and_then(|certs| certs.first().cloned());

            server::serve_tls_connection(tls_stream, peer_addr, client_cert).await;
        }
        Err(e) => {
            kmsg::warn!("TLS handshake failed from {}: {:?}", peer_addr, e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_args_maintenance_flag() {
        let args: Vec<String> = "apid --maintenance"
            .split_whitespace()
            .map(String::from)
            .collect();

        assert!(args.iter().any(|a| a == "--maintenance"));
    }

    #[test]
    fn test_args_struct() {
        let args = Args::new("127.0.0.1:8443".to_string(), true);
        assert_eq!(args.listen_addr, "127.0.0.1:8443");
        assert!(args.maintenance_mode);

        let args2 = Args::new("0.0.0.0:443".to_string(), false);
        assert_eq!(args2.listen_addr, "0.0.0.0:443");
        assert!(!args2.maintenance_mode);
    }

    #[test]
    fn test_args_equality() {
        let args1 = Args::new("127.0.0.1:8443".to_string(), true);
        let args2 = Args::new("127.0.0.1:8443".to_string(), true);
        let args3 = Args::new("127.0.0.1:8443".to_string(), false);

        assert_eq!(args1, args2);
        assert_ne!(args1, args3);
    }
}
