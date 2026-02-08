//! API Gateway Daemon entry point.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};

#[tokio::main]
async fn main() -> Result<()> {
    kmsg::init("apid")?;
    kmsg::info!("API daemon starting");

    sysconfig::init()?;

    let args = apid::parse_args(&std::env::args().collect::<Vec<_>>());
    let notifier = notify::NotifyClient::new("apid")?;

    let addr: std::net::SocketAddr = args.listen_addr.parse()?;
    let listener = TcpListener::bind(addr).await?;

    let tls_acceptor = apid::setup_tls(args.maintenance_mode)?;

    kmsg::info!("API daemon ready, listening on {}", addr);
    notifier.ready(&format!("tcp://{}", args.listen_addr))?;

    let shutdown = setup_shutdown_handler();

    apid::run(&listener, &tls_acceptor, &shutdown).await;

    notifier.stopping("Graceful shutdown")?;
    kmsg::info!("API daemon stopped");

    Ok(())
}

/// Sets up signal handlers for graceful shutdown.
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
