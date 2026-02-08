//! Network daemon for Muak - Manages network interfaces, DHCP, DNS, and connectivity

mod actor;
mod config;
mod connectivity;
mod dhcpv4;
mod dns;
mod grpc;
mod interface;
mod model;
mod monitor;
mod netlink;
mod services;
mod slaac;
mod socket;

use anyhow::Result;
use notify::{Health, NotifyClient};
use std::path::Path;
use tokio::net::UnixListener;
use tokio::signal::unix::{SignalKind, signal};
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;

use actor::start_network_actor;
use grpc::NetworkServiceImpl;

#[allow(clippy::excessive_nesting)]
pub mod proto {
    tonic::include_proto!("muak.internal.network");
}

const SOCKET_PATH: &str = "/run/networkd.sock";

/// Entry point for the network daemon
#[tokio::main]
async fn main() -> Result<()> {
    kmsg::init("networkd")?;
    kmsg::info!("Starting networkd");

    sysconfig::init()?;

    let notifier = NotifyClient::new("networkd")?;
    notifier.status("Initializing network subsystem", Health::Healthy)?;

    let network_handle = start_network_actor().await?;

    notifier.status("Discovering interfaces and acquiring DHCP", Health::Healthy)?;
    network_handle.initialize_with_retry().await?;

    if Path::new(SOCKET_PATH).exists() {
        std::fs::remove_file(SOCKET_PATH)?;
    }

    let listener = UnixListener::bind(SOCKET_PATH)?;
    let stream = UnixListenerStream::new(listener);

    kmsg::info!("Listening on {}", SOCKET_PATH);

    notifier.ready(SOCKET_PATH)?;

    let service = NetworkServiceImpl::new(network_handle);

    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;

    let server = Server::builder()
        .add_service(proto::network_service_server::NetworkServiceServer::new(
            service,
        ))
        .serve_with_incoming_shutdown(stream, async {
            tokio::select! {
                _ = sigterm.recv() => {
                    kmsg::info!("Received SIGTERM, shutting down");
                }
                _ = sigint.recv() => {
                    kmsg::info!("Received SIGINT, shutting down");
                }
            }
        });

    tokio::select! {
        result = server => {
            if let Err(e) = result {
                kmsg::error!("Server error: {}", e);
                return Err(e.into());
            }
        }
    }

    notifier.stopping("Graceful shutdown")?;
    kmsg::info!("Shutdown complete");

    Ok(())
}
