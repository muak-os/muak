mod disk;
mod provisioning;
mod services;
mod supervisor;

use std::path::Path;
use supervisor::{ServiceDef, Supervisor};
use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;

const GRPC_SOCKET_PATH: &str = "/run/granola.sock";
const GRPC_SERVER_ADDR: &str = "0.0.0.0:50051";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    kmsg::init("granola")?;
    kmsg::info!("PID 1 supervisor started");

    let is_installed = match provisioning::status() {
        provisioning::InstallationStatus::Live => {
            kmsg::info!("CURRENTLY IN MAINTENANCE MODE");
            kmsg::info!("   Run 'muakctl install --target <disk>' to install");
            false
        }
        provisioning::InstallationStatus::Installed => {
            kmsg::info!("Running from INSTALLED DISK");

            if let Err(e) = disk::mount_partitions() {
                kmsg::warn!("Failed to mount partitions: {}", e);
                // TODO: set maintenance mode here to recover
                false
            } else if let Err(e) = provisioning::check_and_handle_pending_validation() {
                kmsg::warn!("Update validation handling failed: {}", e);
                true
            } else {
                true
            }
        }
    };

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
            args: vec!["--listen".to_string(), GRPC_SERVER_ADDR.to_string()],
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

    let grpc_handle = tokio::spawn(async {
        if let Err(e) = run_grpc_server().await {
            kmsg::error!("gRPC server error: {}", e);
        }
    });

    supervisor.run().await?;

    grpc_handle.abort();

    Ok(())
}

async fn run_grpc_server() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if Path::new(GRPC_SOCKET_PATH).exists() {
        std::fs::remove_file(GRPC_SOCKET_PATH)?;
    }

    let listener = UnixListener::bind(GRPC_SOCKET_PATH)?;

    Server::builder()
        .add_service(services::process::service())
        .add_service(services::provision::service())
        .serve_with_incoming(UnixListenerStream::new(listener))
        .await?;

    Ok(())
}
