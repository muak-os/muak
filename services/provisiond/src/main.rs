//! Provisioning daemon for Muak.

extern crate alloc;

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

// Mount point of the EPHEMERAL partition.
const EPHEMERAL_MOUNT: &str = "/run/ephemeral";

// Btrfs `f_type` magic reported by statfs.
const BTRFS_MAGIC: i64 = 0x9123_683E;

#[granola::service("provisiond")]
#[tokio::main]
async fn main(notifier: NotifyClient) -> Result<()> {
    config::init().context("Failed to initialize system configuration")?;

    let is_installed = Path::new(config::system::CONFIG_PATH).exists();

    let stream = tokio_stream::wrappers::UnixListenerStream::new(socket()?);

    notifier.ready()?;

    if is_installed {
        if cache_available() {
            update::init_cache();
        } else {
            kmsg::warn!("EPHEMERAL is not a btrfs mount; koci pull cache disabled");
        }

        let _result = update::check_and_handle_pending_validation()
            .map_err(|e| kmsg::warn!("Update validation handling failed: {}", e));
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

fn cache_available() -> bool {
    rustix::fs::statfs(EPHEMERAL_MOUNT).is_ok_and(|stat| stat.f_type == BTRFS_MAGIC)
}
