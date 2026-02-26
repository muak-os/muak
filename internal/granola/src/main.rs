//! PID 1 supervisor for Muak.

mod services;
mod supervisor;

use anyhow::{Context, Result};
use supervisor::logger::LogReader;
use supervisor::{ServiceDef, Supervisor};
use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;

const GRPC_SOCKET_PATH: &str = "/run/granola.sock";

#[tokio::main]
async fn main() -> Result<()> {
    kmsg::init("granola")?;
    kmsg::info!("PID 1 supervisor started");

    sysconfig::init().context("Failed to initialize system configuration")?;

    let mut apid_args = vec![
        "--listen".to_string(),
        format!("0.0.0.0:{}", sysconfig::system().port),
    ];

    let is_installed = std::path::Path::new(sysconfig::CONFIG_PATH).exists();

    if is_installed {
        kmsg::info!("Running from INSTALLED DISK");
    } else {
        kmsg::info!("CURRENTLY IN MAINTENANCE MODE");
        kmsg::info!("   Run 'muakctl install --config <config.toml>' to install");
        apid_args.push("--maintenance".to_string());
    }

    let mut services = vec![
        ServiceDef {
            name: "modd",
            binary: "/sbin/modd",
            args: vec![],
            depends_on: vec![],
        },
        ServiceDef {
            name: "networkd",
            binary: "/sbin/networkd",
            args: vec![],
            depends_on: vec![],
        },
        ServiceDef {
            name: "provisiond",
            binary: "/sbin/provisiond",
            args: vec![],
            depends_on: vec![],
        },
        ServiceDef {
            name: "timed",
            binary: "/sbin/timed",
            args: vec![],
            depends_on: vec!["networkd"],
        },
        ServiceDef {
            name: "apid",
            binary: "/sbin/apid",
            args: apid_args,
            depends_on: vec!["networkd"],
        },
    ];

    if is_installed {
        services.push(ServiceDef {
            name: "vmd",
            binary: "/sbin/vmd",
            args: vec![],
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
    if std::path::Path::new(GRPC_SOCKET_PATH).exists() {
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
