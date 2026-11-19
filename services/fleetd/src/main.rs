//! Supervisor entry point for the optional fleet extension.

use granola::runtime::socket::socket;

#[granola::service("fleetd")]
#[tokio::main]
async fn main(notifier: NotifyClient) -> Result<()> {
    config::init()?;
    let config = config::system::config()?;
    let listener = socket()?;
    notifier.ready()?;
    fleetd::run(listener, config.fleet.as_ref(), config.host.name.clone()).await
}
