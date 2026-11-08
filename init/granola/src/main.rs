//! PID 1 supervisor for Muak.

extern crate alloc;

mod ipc;
mod loader;
mod supervisor;

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context as _, Result};
use supervisor::logger::LogReader;
use supervisor::{Supervisor, status};
use tokio::net::UnixListener;
use tokio::sync::watch;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;

const GRPC_SOCKET_PATH: &str = "/run/services/granola.sock";

#[tokio::main]
async fn main() -> Result<()> {
    kmsg::init("granola")?;
    kmsg::info!("PID 1 supervisor started");

    let maintenance = maintenance_from_env();

    if maintenance == "false" {
        kmsg::info!("Running from INSTALLED DISK");
    } else {
        kmsg::info!("!!! CURRENTLY IN MAINTENANCE MODE !!!");
    }

    let mut env: HashMap<&str, &str> = HashMap::new();
    env.insert("MAINTENANCE", maintenance);

    let services = loader::load(&env).context("Failed to load service definitions")?;

    let (writer, reader, actor) = supervisor::logger::create();

    tokio::spawn(actor.run());

    supervisor::logger::sources::kmsg(&writer);

    let (status, status_rx) = supervisor::status::channel();
    let mut supervisor = Supervisor::new(services, writer, status)?;

    tokio::spawn(grpc_server_task(reader, status_rx));

    supervisor.run().await
}

/// Logs gRPC server failures without taking the supervisor down.
async fn grpc_server_task(reader: LogReader, status_rx: watch::Receiver<status::Snapshot>) {
    if let Err(e) = run_grpc_server(reader, status_rx).await {
        kmsg::error!("gRPC server error: {e}");
    }
}

/// Runs the gRPC server for internal service communication.
async fn run_grpc_server(
    reader: LogReader,
    status_rx: watch::Receiver<status::Snapshot>,
) -> Result<()> {
    if Path::new(GRPC_SOCKET_PATH).exists() {
        std::fs::remove_file(GRPC_SOCKET_PATH)?;
    }

    let listener = UnixListener::bind(GRPC_SOCKET_PATH)?;

    Server::builder()
        .add_service(ipc::process::service())
        .add_service(ipc::log::service(reader))
        .add_service(ipc::services::service(status_rx))
        .serve_with_incoming(UnixListenerStream::new(listener))
        .await?;

    Ok(())
}

fn maintenance_from_env() -> &'static str {
    maintenance(std::env::var("MAINTENANCE").ok().as_deref())
}

fn maintenance(raw: Option<&str>) -> &'static str {
    if raw == Some("true") { "true" } else { "false" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maintenance_true_passthrough() {
        // ARRANGE
        let raw = Some("true");

        // ACT
        let flag = maintenance(raw);

        // ASSERT
        assert_eq!(flag, "true");
    }

    #[test]
    fn maintenance_missing_is_false() {
        // ARRANGE
        let raw = None;

        // ACT
        let flag = maintenance(raw);

        // ASSERT
        assert_eq!(flag, "false");
    }

    #[test]
    fn maintenance_false_passthrough() {
        // ARRANGE
        let raw = Some("false");

        // ACT
        let flag = maintenance(raw);

        // ASSERT
        assert_eq!(flag, "false");
    }

    #[test]
    fn maintenance_garbage_is_false() {
        // ARRANGE
        let raw = Some("garbage");

        // ACT
        let flag = maintenance(raw);

        // ASSERT
        assert_eq!(flag, "false");
    }
}
