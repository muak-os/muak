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
use tonic::transport::Server;

#[granola::service("provisiond")]
#[tokio::main]
async fn main(notifier: NotifyClient) -> Result<()> {
    update::resolve_pending_config();

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
