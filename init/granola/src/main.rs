//! PID 1 supervisor for Muak.

extern crate alloc;

mod ipc;
mod loader;
mod supervisor;

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

    let env = std::env::vars_os()
        .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)))
        .collect();

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
