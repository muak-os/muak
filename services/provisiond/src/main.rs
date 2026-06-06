//! Provisioning daemon for Muak.

mod constants;
mod disk;
mod efi;
mod history;
mod install;
mod ipc;
mod profile;
mod reboot;
mod reset;
mod secrets;
mod streaming;
mod uki;
mod update;

use std::path::Path;

use anyhow::Context;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;

#[granola::service("provisiond")]
#[tokio::main]
async fn main(notifier: NotifyClient) -> Result<()> {
    config::init().context("Failed to initialize system configuration")?;

    let is_installed = Path::new(config::CONFIG_PATH).exists();
    if is_installed {
        let _ = update::check_and_handle_pending_validation()
            .map_err(|e| kmsg::warn!("Update validation handling failed: {}", e));
    }

    let stream = UnixListenerStream::new(granola::socket()?);

    notifier.ready()?;

    Server::builder()
        .add_service(ipc::auth::service())
        .add_service(ipc::provision::service())
        .add_service(ipc::security::service())
        .add_service(ipc::version::service())
        .serve_with_incoming_shutdown(stream, granola::shutdown_signal())
        .await?;

    Ok(())
}
