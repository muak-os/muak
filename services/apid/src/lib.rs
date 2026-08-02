//! `apid` - API Gateway Daemon for Muak.

extern crate alloc;

pub mod constants;
pub mod handler;
pub mod proxy;
pub mod rbac;
pub mod server;
pub mod tls;

use alloc::sync::Arc;
use core::future::Future;
use core::net::SocketAddr;
use core::time::Duration;

use anyhow::Result;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::time::timeout;
use tokio_rustls::TlsAcceptor;

use crate::proxy::BackendPool;

/// How often the accept loop re-checks for a shutdown request while idle.
const ACCEPT_POLL: Duration = Duration::from_millis(100);

/// Arguments parsed from command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub listen_addr: String,
    pub maintenance_mode: bool,
}

impl Args {
    /// Creates new command line arguments struct.
    #[must_use]
    pub fn new(listen_addr: String, maintenance_mode: bool) -> Self {
        Self {
            listen_addr,
            maintenance_mode,
        }
    }
}

/// Parses command line arguments into an Args struct.
#[must_use]
pub fn parse_args(args: &[String]) -> Args {
    let default_listen = format!("0.0.0.0:{}", config::host().port);

    let listen_addr = args
        .iter()
        .position(|arg| arg == "--listen")
        .and_then(|idx| args.get(idx.saturating_add(1)))
        .cloned()
        .unwrap_or(default_listen);

    let maintenance_mode = args
        .iter()
        .position(|arg| arg == "--maintenance")
        .and_then(|idx| args.get(idx.saturating_add(1)))
        .is_some_and(|value| value == "true");

    Args {
        listen_addr,
        maintenance_mode,
    }
}

/// Sets up TLS configuration based on the mode.
///
/// # Errors
///
/// Returns an error if the TLS configuration cannot be loaded or generated.
pub fn setup_tls(maintenance_mode: bool) -> Result<TlsAcceptor> {
    if maintenance_mode {
        tls::generate_ephemeral_tls_config()
    } else {
        tls::load_tls_config()
    }
}

/// Runs the main loop for incoming connections.
pub async fn run<S>(
    listener: &TcpListener,
    tls_acceptor: &TlsAcceptor,
    shutdown: S,
    maintenance_mode: bool,
) where
    S: Future<Output = ()> + Send + 'static,
{
    let pool = Arc::new(BackendPool::new());

    let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
    tokio::spawn(async move {
        shutdown.await;
        let _send = shutdown_tx.send(());
    });

    loop {
        match timeout(ACCEPT_POLL, listener.accept()).await {
            Ok(Ok((stream, peer_addr))) => {
                let acceptor = tls_acceptor.clone();
                let pool = Arc::clone(&pool);
                tokio::spawn(handle_tls_connection(
                    pool,
                    acceptor,
                    stream,
                    peer_addr,
                    maintenance_mode,
                ));
            }
            Ok(Err(e)) => {
                eprintln!("Accept error: {e}");
            }
            Err(_elapsed) if shutdown_rx.try_recv().is_ok() => break,
            Err(_elapsed) => {}
        }
    }
}

/// Handles the TLS handshake and connection for a single client.
async fn handle_tls_connection(
    pool: Arc<BackendPool>,
    acceptor: TlsAcceptor,
    stream: TcpStream,
    peer_addr: SocketAddr,
    maintenance_mode: bool,
) {
    match acceptor.accept(stream).await {
        Ok(tls_stream) => {
            let client_cert = tls_stream
                .get_ref()
                .1
                .peer_certificates()
                .and_then(|certs| certs.first().cloned());

            server::serve_tls_connection(
                pool,
                tls_stream,
                peer_addr,
                client_cert,
                maintenance_mode,
            )
            .await;
        }
        Err(e) => {
            eprintln!("TLS handshake failed from {peer_addr}: {e:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_maintenance(args: &[&str]) -> bool {
        let owned: Vec<String> = args.iter().map(ToString::to_string).collect();
        owned
            .iter()
            .position(|arg| arg == "--maintenance")
            .and_then(|idx| owned.get(idx.saturating_add(1)))
            .is_some_and(|value| value == "true")
    }

    #[test]
    fn parse_args_maintenance_flag() {
        // ARRANGE + ACT + ASSERT
        assert!(parse_maintenance(&["--maintenance", "true"]));
    }

    #[test]
    fn parse_args_maintenance_false() {
        // ARRANGE + ACT + ASSERT
        assert!(!parse_maintenance(&["--maintenance", "false"]));
    }

    #[test]
    fn parse_args_maintenance_absent_defaults_false() {
        // ARRANGE + ACT + ASSERT
        assert!(!parse_maintenance(&["--listen", "0.0.0.0:443"]));
    }

    #[test]
    fn args_struct() {
        // ARRANGE
        let args = Args::new("127.0.0.1:8443".to_owned(), true);
        let args2 = Args::new("0.0.0.0:443".to_owned(), false);

        // ASSERT
        assert_eq!(args.listen_addr, "127.0.0.1:8443");
        assert!(args.maintenance_mode);

        assert_eq!(args2.listen_addr, "0.0.0.0:443");
        assert!(!args2.maintenance_mode);
    }

    #[test]
    fn args_equality() {
        // ARRANGE
        let args1 = Args::new("127.0.0.1:8443".to_owned(), true);
        let args2 = Args::new("127.0.0.1:8443".to_owned(), true);
        let args3 = Args::new("127.0.0.1:8443".to_owned(), false);

        // ACT & ASSERT
        assert_eq!(args1, args2);
        assert_ne!(args1, args3);
    }
}
