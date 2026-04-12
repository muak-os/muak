//! Network daemon for Muak to manage network interfaces, DHCP & DNS

mod dhcp;
mod dns;
mod interface;
mod monitor;
mod slaac;
mod snapshot;
mod state_machine;
mod supervisor;

use granola::Health;
use supervisor::start_network_supervisor;

#[granola::service("networkd")]
#[tokio::main]
async fn main(notifier: NotifyClient) -> Result<()> {
    config::init()?;

    notifier.status("Initializing network subsystem", Health::Healthy)?;

    let network_handle = start_network_supervisor().await?;

    notifier.status("Discovering interfaces and acquiring DHCP", Health::Healthy)?;
    network_handle.initialize_with_retry().await?;

    notifier.ready()?;

    granola::shutdown_signal().await;

    Ok(())
}
