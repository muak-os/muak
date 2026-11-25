//! Network daemon for Muak to manage network interfaces, DHCP & DNS.

use std::time::Duration;

use granola::runtime::notify::Health;
use net_runtime_linux::Tokio;
use networkd::config::convert;
use networkd::service::{DEGRADED_STATUS, Readiness, Service};

const READY_TIMEOUT: Duration = Duration::from_secs(6);

#[granola::service("networkd")]
#[tokio::main]
async fn main(notifier: NotifyClient) -> Result<()> {
    config::init()?;

    notifier.status("Initializing network subsystem", Health::Healthy)?;

    let mut service = Service::new(Tokio::new()?, convert(config::system::network()?));

    service.start().await?;

    notifier.status("Setting up network", Health::Healthy)?;

    match service.wait_ready(READY_TIMEOUT).await? {
        Readiness::Ready => notifier.ready()?,
        Readiness::Degraded => {
            notifier.status(DEGRADED_STATUS, Health::Degraded)?;
            notifier.ready()?;
        }
        Readiness::Shutdown => return Ok(()),
    }

    service.run(&notifier).await?;

    Ok(())
}
