mod auth;
mod config;
mod handler;
mod proxy;
mod server;
mod tls;

use anyhow::Result;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};

#[tokio::main]
async fn main() -> Result<()> {
    kmsg::init("apid")?;
    kmsg::info!("API daemon starting");

    sysconfig::init()?;

    let (listen_addr, maintenance_mode) = parse_args();
    let notifier = notify::NotifyClient::new("apid")?;

    let addr: SocketAddr = listen_addr.parse()?;
    let listener = TcpListener::bind(addr).await?;

    let tls_acceptor = if maintenance_mode {
        tls::generate_ephemeral_tls_config()?
    } else {
        tls::load_tls_config()?
    };

    kmsg::info!("API daemon ready, listening on {}", addr);
    notifier.ready(&format!("tcp://{}", listen_addr))?;

    let shutdown = setup_shutdown_handler();

    run_accept_loop(&listener, &tls_acceptor, &shutdown).await;

    notifier.stopping("Graceful shutdown")?;
    kmsg::info!("API daemon stopped");

    Ok(())
}

/// Parses command line arguments
fn parse_args() -> (String, bool) {
    let args: Vec<String> = std::env::args().collect();
    let default_listen = format!("0.0.0.0:{}", sysconfig::system().port);

    let listen_addr = args
        .iter()
        .position(|a| a == "--listen")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or(default_listen);

    let maintenance_mode = args.iter().any(|a| a == "--maintenance");

    (listen_addr, maintenance_mode)
}

fn setup_shutdown_handler() -> Arc<AtomicBool> {
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = shutdown.clone();

    tokio::spawn(async move {
        let mut sigterm = signal(SignalKind::terminate()).ok();
        let mut sigint = signal(SignalKind::interrupt()).ok();

        tokio::select! {
            _ = async { sigterm.as_mut()?.recv().await }, if sigterm.is_some() => {
                kmsg::info!("Received SIGTERM, shutting down");
            }
            _ = async { sigint.as_mut()?.recv().await }, if sigint.is_some() => {
                kmsg::info!("Received SIGINT, shutting down");
            }
        }
        shutdown_clone.store(true, Ordering::SeqCst);
    });

    shutdown
}

async fn run_accept_loop(
    listener: &TcpListener,
    tls_acceptor: &tokio_rustls::TlsAcceptor,
    shutdown: &Arc<AtomicBool>,
) {
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

async fn handle_tls_connection(
    acceptor: tokio_rustls::TlsAcceptor,
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
