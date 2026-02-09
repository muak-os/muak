//! PID 1 supervisor and init system for Muak.

mod services;
mod supervisor;

use anyhow::{Context, Result};
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
            name: "modd".to_string(),
            binary: "/sbin/modd".to_string(),
            args: vec![],
            depends_on: vec![],
        },
        ServiceDef {
            name: "networkd".to_string(),
            binary: "/sbin/networkd".to_string(),
            args: vec![],
            depends_on: vec![],
        },
        ServiceDef {
            name: "provisiond".to_string(),
            binary: "/sbin/provisiond".to_string(),
            args: vec![],
            depends_on: vec![],
        },
        ServiceDef {
            name: "apid".to_string(),
            binary: "/sbin/apid".to_string(),
            args: apid_args,
            depends_on: vec!["networkd".to_string(), "provisiond".to_string()],
        },
    ];

    if is_installed {
        services.push(ServiceDef {
            name: "vmd".to_string(),
            binary: "/sbin/vmd".to_string(),
            args: vec![],
            depends_on: vec!["networkd".to_string()],
        });
    } else {
        kmsg::info!("VM service disabled in maintenance mode");
    }

    let mut supervisor = Supervisor::new(services)?;

    tokio::spawn(async {
        if let Err(e) = run_grpc_server().await {
            kmsg::error!("gRPC server error: {}", e);
        }
    });

    supervisor.run().await?;

    unreachable!("If we're here, something went very wrong");
}

/// Runs the gRPC server for internal service communication.
async fn run_grpc_server() -> Result<()> {
    if std::path::Path::new(GRPC_SOCKET_PATH).exists() {
        std::fs::remove_file(GRPC_SOCKET_PATH)?;
    }

    let listener = UnixListener::bind(GRPC_SOCKET_PATH)?;

    Server::builder()
        .add_service(services::process::service())
        .serve_with_incoming(UnixListenerStream::new(listener))
        .await?;

    Ok(())
}
