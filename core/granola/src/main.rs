//! PID 1 supervisor for Muak.

mod ipc;
mod loader;
mod runtime;
mod supervisor;

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use supervisor::Supervisor;
use supervisor::logger::LogReader;
use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;

const GRPC_SOCKET_PATH: &str = "/run/services/granola.sock";

#[tokio::main]
async fn main() -> Result<()> {
    kmsg::init("granola")?;
    kmsg::info!("PID 1 supervisor started");

    config::init().context("Failed to initialize system configuration")?;

    let installed = Path::new(config::CONFIG_PATH).exists();

    if installed {
        kmsg::info!("Running from INSTALLED DISK");
    } else {
        kmsg::info!("!!! CURRENTLY IN MAINTENANCE MODE !!!");
    }

    let port_str = config::host().port.to_string();
    let mut env: HashMap<&str, &str> = HashMap::new();
    env.insert("PORT", &port_str);
    env.insert("MAINTENANCE", if installed { "false" } else { "true" });

    let services = loader::load(&env).context("Failed to load service definitions")?;

    let (writer, reader, actor) = supervisor::logger::create();

    tokio::spawn(actor.run());

    supervisor::logger::kmsg(&writer);

    let mut supervisor = Supervisor::new(services, writer)?;

    tokio::spawn(async {
        if let Err(e) = run_grpc_server(reader).await {
            kmsg::error!("gRPC server error: {}", e);
        }
    });

    supervisor.run().await?;

    unreachable!("If we're here, something went very wrong")
}

/// Runs the gRPC server for internal service communication.
async fn run_grpc_server(reader: LogReader) -> Result<()> {
    if Path::new(GRPC_SOCKET_PATH).exists() {
        std::fs::remove_file(GRPC_SOCKET_PATH)?;
    }

    let listener = UnixListener::bind(GRPC_SOCKET_PATH)?;

    Server::builder()
        .add_service(ipc::process::service())
        .add_service(ipc::log::service(reader))
        .serve_with_incoming(UnixListenerStream::new(listener))
        .await?;

    Ok(())
}
