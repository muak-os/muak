//! Provisioning daemon for Muak.

extern crate alloc;

mod cache;
mod check;
mod disk;
mod efi;
mod install;
mod ipc;
mod journal;
mod medium;
mod power;
mod reset;
mod secrets;
mod streaming;
mod template;
mod update;

use std::path::Path;

use anyhow::Context as _;
use granola::runtime::signal::shutdown;
use granola::runtime::socket::socket;
use rustix::system::RebootCommand;
use tonic::transport::Server;

#[granola::service("provisiond")]
#[tokio::main]
async fn main(notifier: NotifyClient) -> Result<()> {
    restore_unreadable_config();

    config::init().context("Failed to initialize system configuration")?;

    let is_installed = Path::new(config::system::CONFIG_PATH).exists();

    let stream = tokio_stream::wrappers::UnixListenerStream::new(socket()?);

    notifier.ready()?;

    if is_installed {
        if cache::available() {
            cache::init();
            cache::enforce();
        } else {
            kmsg::warn!("pull cache filesystem is not btrfs; koci cache GC disabled");
        }

        update::check_and_handle_pending_validation();
    }

    Server::builder()
        .add_service(ipc::auth::service())
        .add_service(ipc::provision::service())
        .add_service(ipc::security::service())
        .add_service(ipc::version::service())
        .serve_with_incoming_shutdown(stream, shutdown())
        .await?;

    Ok(())
}

fn restore_unreadable_config() {
    if !Path::new(update::snapshot::CONFIG_PREV_PATH).exists() {
        return;
    }

    let Err(error) = config::system::load_from_path(Path::new(config::system::CONFIG_PATH)) else {
        return;
    };

    kmsg::error!("Installed config is unreadable ({error:#}); restoring the previous config");
    if let Err(error) = std::fs::rename(
        update::snapshot::CONFIG_PREV_PATH,
        config::system::CONFIG_PATH,
    ) {
        kmsg::error!("Failed to restore the previous config: {error}");
        return;
    }

    rustix::fs::sync();
    kmsg::info!("Previous config restored; rebooting to retry the boot");

    if let Err(error) = rustix::system::reboot(RebootCommand::Restart) {
        kmsg::error!("Failed to reboot after restoring the previous config: {error}");
    }
}
