//! Network daemon for Muak to manage network interfaces, DHCP & DNS

use granola::Health;

#[granola::service("networkd")]
#[tokio::main]
async fn main(notifier: NotifyClient) -> Result<()> {
    config::init()?;

    notifier.status("Initializing network subsystem", Health::Healthy)?;

    let handle = networkd::supervisor::start().await?;

    notifier.status("Setting up network", Health::Healthy)?;
    handle.initialize_with_retry().await?;

    notifier.ready()?;

    granola::shutdown_signal().await;

    Ok(())
}
