//! Provisioning daemon for Muak.

mod constants;
mod disk;
mod efi;
mod history;
mod install;
mod reboot;
mod reset;
mod secrets;
mod services;
mod streaming;
mod uki;
mod update;

use std::os::unix::io::FromRawFd;
use std::os::unix::net::UnixListener as StdUnixListener;
use std::path::Path;

use anyhow::{Context, Result};
use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;

#[tokio::main]
async fn main() -> Result<()> {
    kmsg::init("provisiond")?;
    kmsg::info!("Provisioning daemon started");

    sysconfig::init().context("Failed to initialize system configuration")?;

    let is_installed = Path::new(sysconfig::CONFIG_PATH).exists();

    if is_installed {
        let _ = update::check_and_handle_pending_validation()
            .await
            .map_err(|e| kmsg::warn!("Update validation handling failed: {}", e));
    }

    // SAFETY: granola pre-binds the socket and passes it as FD 3 before exec.
    let std_listener = unsafe { StdUnixListener::from_raw_fd(3) };
    std_listener
        .set_nonblocking(true)
        .context("Failed to set listener non-blocking")?;
    let listener = UnixListener::from_std(std_listener).context("Failed to create UnixListener")?;

    let notifier =
        notify::NotifyClient::new("provisiond").context("Failed to create notify client")?;
    notifier
        .ready()
        .context("Failed to send ready notification")?;

    kmsg::info!("provisiond started");

    Server::builder()
        .add_service(services::auth::service())
        .add_service(services::provision::service())
        .add_service(services::security::service())
        .serve_with_incoming(UnixListenerStream::new(listener))
        .await?;

    Ok(())
}
