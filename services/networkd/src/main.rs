//! Network daemon for Muak to manage network interfaces, DHCP & DNS

mod actor;
mod dhcp;
mod dns;
mod ipc;
mod monitor;
mod slaac;

use actor::start_network_actor;
use granola::Health;
use ipc::NetworkServiceImpl;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;

#[allow(clippy::excessive_nesting)]
pub mod proto {
    tonic::include_proto!("muak.internal.network");
}

#[granola::service("networkd")]
#[tokio::main]
async fn main(notifier: NotifyClient) -> Result<()> {
    config::init()?;

    notifier.status("Initializing network subsystem", Health::Healthy)?;

    let network_handle = start_network_actor().await?;

    notifier.status("Discovering interfaces and acquiring DHCP", Health::Healthy)?;
    network_handle.initialize_with_retry().await?;

    let stream = UnixListenerStream::new(granola::socket()?);

    notifier.ready()?;

    Server::builder()
        .add_service(proto::network_service_server::NetworkServiceServer::new(
            NetworkServiceImpl::new(network_handle),
        ))
        .serve_with_incoming_shutdown(stream, granola::shutdown_signal())
        .await?;

    Ok(())
}
