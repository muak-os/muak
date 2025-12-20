mod services;

use notify::NotifyClient;
use std::net::SocketAddr;
use tokio::signal::unix::{SignalKind, signal};
use tonic::transport::Server;

const DEFAULT_LISTEN_ADDR: &str = "0.0.0.0:50051";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    kmsg::init("apid")?;
    kmsg::info!("API daemon starting");

    let args: Vec<String> = std::env::args().collect();
    let listen_addr = args
        .iter()
        .position(|a| a == "--listen")
        .and_then(|i| args.get(i + 1))
        .map(|s| s.as_str())
        .unwrap_or(DEFAULT_LISTEN_ADDR);

    let notifier = NotifyClient::new("apid")?;

    let addr: SocketAddr = listen_addr.parse()?;
    kmsg::info!("API daemon ready, listening on {}", addr);
    notifier.ready(&format!("tcp://{}", listen_addr))?;

    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;

    let server = Server::builder()
        .add_service(services::process::service())
        .add_service(services::vm::service())
        .add_service(services::provision::service())
        .serve_with_shutdown(addr, async {
            tokio::select! {
                _ = sigterm.recv() => {
                    kmsg::info!("Received SIGTERM, shutting down");
                }
                _ = sigint.recv() => {
                    kmsg::info!("Received SIGINT, shutting down");
                }
            }
        });

    if let Err(e) = server.await {
        kmsg::error!("gRPC server error: {}", e);
        notifier.stopping(&format!("Server error: {}", e))?;
        return Err(e.into());
    }

    notifier.stopping("Graceful shutdown")?;
    kmsg::info!("API daemon stopped");

    Ok(())
}
