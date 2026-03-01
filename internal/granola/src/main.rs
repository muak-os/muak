//! PID 1 supervisor for Muak.

mod services;
mod supervisor;

use std::path::Path;

use anyhow::{Context, Result};
use supervisor::logger::LogReader;
use supervisor::{ServiceDef, Supervisor};
use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;

const GRPC_SOCKET_PATH: &str = "/run/services/granola.sock";

#[tokio::main]
async fn main() -> Result<()> {
    kmsg::init("granola")?;
    kmsg::info!("PID 1 supervisor started");

    sysconfig::init().context("Failed to initialize system configuration")?;

    let mut apid_command = vec![
        "/sbin/apid".to_string(),
        "--listen".to_string(),
        format!("0.0.0.0:{}", sysconfig::system().port),
    ];

    let is_installed = Path::new(sysconfig::CONFIG_PATH).exists();

    if is_installed {
        kmsg::info!("Running from INSTALLED DISK");
    } else {
        kmsg::info!("!!! CURRENTLY IN MAINTENANCE MODE !!!");
        apid_command.push("--maintenance".to_string());
    }

    let mut services = vec![
        ServiceDef {
            name: "modd",
            command: vec!["/sbin/modd".to_string()],
            depends_on: vec![],
        },
        ServiceDef {
            name: "networkd",
            command: vec!["/sbin/networkd".to_string()],
            depends_on: vec![],
        },
        ServiceDef {
            name: "provisiond",
            command: vec!["/sbin/provisiond".to_string()],
            depends_on: vec![],
        },
        ServiceDef {
            name: "timed",
            command: vec!["/sbin/timed".to_string()],
            depends_on: vec!["networkd"],
        },
        ServiceDef {
            name: "apid",
            command: apid_command,
            depends_on: vec!["networkd"],
        },
    ];

    if is_installed {
        services.push(ServiceDef {
            name: "vmd",
            command: vec!["/sbin/vmd".to_string()],
            depends_on: vec!["networkd"],
        });
    } else {
        kmsg::info!("VM service disabled in maintenance mode");
    }

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

    unreachable!("If we're here, something went very wrong");
}

/// Runs the gRPC server for internal service communication.
async fn run_grpc_server(reader: LogReader) -> Result<()> {
    if Path::new(GRPC_SOCKET_PATH).exists() {
        std::fs::remove_file(GRPC_SOCKET_PATH)?;
    }

    let listener = UnixListener::bind(GRPC_SOCKET_PATH)?;

    Server::builder()
        .add_service(services::process::service())
        .add_service(services::log::service(reader))
        .serve_with_incoming(UnixListenerStream::new(listener))
        .await?;

    Ok(())
}
