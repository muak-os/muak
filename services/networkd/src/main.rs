//! Network daemon for Muak to manage network interfaces, DHCP & DNS.

use granola::runtime::notify::Health;
use granola::runtime::signal::shutdown;
use networkd::supervisor;

#[granola::service("networkd")]
#[tokio::main]
async fn main(notifier: NotifyClient) -> Result<()> {
    config::init()?;

    notifier.status("Initializing network subsystem", Health::Healthy)?;

    let handle = supervisor::start()?;

    notifier.status("Setting up network", Health::Healthy)?;
    handle.initialize_with_retry().await?;

    notifier.ready()?;

    shutdown().await;

    Ok(())
}
