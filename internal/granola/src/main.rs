mod disk;
mod provisioning;
mod services;
mod supervisor;

use anyhow::Result;
use std::path::Path;
use supervisor::{ServiceDef, Supervisor};
use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;

const GRPC_SOCKET_PATH: &str = "/run/granola.sock";

#[tokio::main]
async fn main() -> Result<()> {
    kmsg::init("granola")?;
    kmsg::info!("PID 1 supervisor started");

    sysconfig::init()?;

    let mut is_installed = matches!(
        provisioning::status(),
        provisioning::InstallationStatus::Installed
    );

    if is_installed {
        kmsg::info!("Running from INSTALLED DISK");
    } else {
        kmsg::info!("CURRENTLY IN MAINTENANCE MODE");
        kmsg::info!("   Run 'muakctl install --config <config.toml>' to install");
    }

    if is_installed && disk::mount_partitions().is_err() {
        let _ =
            disk::mount_partitions().map_err(|e| kmsg::warn!("Failed to mount partitions: {}", e));
        is_installed = false;
        // TODO: set in maintenance mode here to recover
    }

    if is_installed {
        let _ = provisioning::check_and_handle_pending_validation()
            .map_err(|e| kmsg::warn!("Update validation handling failed: {}", e));
    }

    let mut apid_args = vec![
        "--listen".to_string(),
        format!("0.0.0.0:{}", sysconfig::system().port),
    ];

    if !is_installed {
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
            name: "apid".to_string(),
            binary: "/sbin/apid".to_string(),
            args: apid_args,
            depends_on: vec!["networkd".to_string()],
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

async fn run_grpc_server() -> Result<()> {
    if Path::new(GRPC_SOCKET_PATH).exists() {
        std::fs::remove_file(GRPC_SOCKET_PATH)?;
    }

    let listener = UnixListener::bind(GRPC_SOCKET_PATH)?;

    Server::builder()
        .add_service(services::auth::service())
        .add_service(services::process::service())
        .add_service(services::provision::service())
        .serve_with_incoming(UnixListenerStream::new(listener))
        .await?;

    Ok(())
}
