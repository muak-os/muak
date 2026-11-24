//! Network daemon for Muak to manage network interfaces, DHCP & DNS.

use granola::runtime::notify::Health;
use net_runtime_linux::Tokio;
use networkd::config::convert;
use networkd::service::Service;

#[granola::service("networkd")]
#[tokio::main]
async fn main(notifier: NotifyClient) -> Result<()> {
    config::init()?;

    notifier.status("Initializing network subsystem", Health::Healthy)?;

    let mut service = Service::new(Tokio::new()?, convert(config::system::network()?));

    service.start().await?;

    notifier.status("Setting up network", Health::Healthy)?;

    if !service.wait_ready().await? {
        return Ok(());
    }

    notifier.ready()?;

    service.run().await?;

    Ok(())
}
