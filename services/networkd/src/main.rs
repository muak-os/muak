//! Network daemon for Muak to manage network interfaces, DHCP & DNS

mod actor;
mod dhcp;
mod dns;
mod monitor;
mod slaac;

use actor::start_network_actor;
use granola::Health;

#[granola::service("networkd")]
#[tokio::main]
async fn main(notifier: NotifyClient) -> Result<()> {
    config::init()?;

    notifier.status("Initializing network subsystem", Health::Healthy)?;

    let network_handle = start_network_actor().await?;

    notifier.status("Discovering interfaces and acquiring DHCP", Health::Healthy)?;
    network_handle.initialize_with_retry().await?;

    notifier.ready()?;

    granola::shutdown_signal().await;

    Ok(())
}
