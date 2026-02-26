//! Provisioning daemon for Muak.
//!
//! Handles all provisioning operations including installation, updates,
//! certificate management, and security state queries.

mod constants;
mod disk;
mod install;
mod reset;
mod services;
mod uki;
mod update;
mod validation;

use anyhow::{Context, Result};
use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;

const GRPC_SOCKET_PATH: &str = "/run/provisiond.sock";

#[tokio::main]
async fn main() -> Result<()> {
    kmsg::init("provisiond")?;
    kmsg::info!("Provisioning daemon started");

    sysconfig::init().context("Failed to initialize system configuration")?;

    let is_installed = std::path::Path::new(sysconfig::CONFIG_PATH).exists();

    if is_installed {
        let _ = validation::check_and_handle_pending_validation()
            .await
            .map_err(|e| kmsg::warn!("Update validation handling failed: {}", e));
    }

    if std::path::Path::new(GRPC_SOCKET_PATH).exists() {
        std::fs::remove_file(GRPC_SOCKET_PATH)?;
    }

    let listener = UnixListener::bind(GRPC_SOCKET_PATH)?;

    let notifier =
        notify::NotifyClient::new("provisiond").context("Failed to create notify client")?;
    notifier
        .ready(GRPC_SOCKET_PATH)
        .context("Failed to send ready notification")?;

    println!("gRPC server listening on {}", GRPC_SOCKET_PATH);

    Server::builder()
        .add_service(services::auth::service())
        .add_service(services::provision::service())
        .add_service(services::security::service())
        .serve_with_incoming(UnixListenerStream::new(listener))
        .await?;

    Ok(())
}
