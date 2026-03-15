//! Network daemon for Muak to manage network interfaces, DHCP, DNS, and connectivity

mod actor;
mod connectivity;
mod dhcpv4;
mod dns;
mod interface;
mod ipc;
mod model;
mod monitor;
mod netlink;
mod netutil;
mod slaac;
mod socket;

use std::os::unix::io::FromRawFd;
use std::os::unix::net::UnixListener as StdUnixListener;

use actor::start_network_actor;
use anyhow::Result;
use ipc::NetworkServiceImpl;
use notify::{Health, NotifyClient};
use tokio::net::UnixListener;
use tokio::signal::unix::{SignalKind, signal};
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;

#[allow(clippy::excessive_nesting)]
pub mod proto {
    tonic::include_proto!("muak.internal.network");
}

#[tokio::main]
async fn main() -> Result<()> {
    kmsg::init("networkd")?;
    kmsg::info!("Starting networkd");

    config::init()?;

    let notifier = NotifyClient::new("networkd")?;
    notifier.status("Initializing network subsystem", Health::Healthy)?;

    let network_handle = start_network_actor().await?;

    notifier.status("Discovering interfaces and acquiring DHCP", Health::Healthy)?;
    network_handle.initialize_with_retry().await?;

    // SAFETY: granola pre-binds the socket and passes it as FD 3 before exec.
    let std_listener = unsafe { StdUnixListener::from_raw_fd(3) };
    std_listener.set_nonblocking(true)?;
    let listener = UnixListener::from_std(std_listener)?;
    let stream = UnixListenerStream::new(listener);

    notifier.ready()?;
    kmsg::info!("networkd started");

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
    println!("Shutdown complete");

    Ok(())
}
