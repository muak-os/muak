//! Optional fleet inventory, isolated from local VM mutation.

extern crate alloc;

mod agent;
mod convert;
mod ipc;
mod manager;
mod model;
mod storage;
#[cfg(test)]
mod tests;

use alloc::sync::Arc;
use std::path::Path;
use std::sync::Mutex;

use anyhow::Result;
use config::system::fleet::Config;
use fleet_api::fleet::fleet_service_server::FleetServiceServer;
use fleet_api::fleet::local_inventory_service_server::LocalInventoryServiceServer;
use granola::runtime::signal::shutdown;
use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;

/// Serves the optional role using the supervisor's management listener.
///
/// # Errors
///
/// Returns errors for invalid or inaccessible state, sockets or configuration.
pub async fn run(listener: UnixListener, config: Option<&Config>, hostname: String) -> Result<()> {
    let Some(config) = config else {
        return serve(
            listener,
            ipc::Service {
                manager: None,
                agent: None,
            },
        )
        .await;
    };
    config.validate()?;
    let (service, worker, publisher) = match *config {
        Config::Manager {
            ref max_nodes,
            ref stale_after_secs,
        } => {
            let ca_pem = std::fs::read_to_string("/run/state/secrets/ca.crt")?;
            let manager = manager::Manager::open(
                Path::new("/run/state/fleet/manager.json"),
                *max_nodes,
                *stale_after_secs,
                ca_pem,
            )?;
            (
                ipc::Service {
                    manager: Some(Arc::new(Mutex::new(manager))),
                    agent: None,
                },
                None,
                None,
            )
        }
        Config::Agent { .. } => {
            let agent =
                agent::Agent::open(Path::new("/run/state/fleet/agent.json"), config, hostname)?;
            let shared = Arc::clone(&agent.shared);
            // This path is owned by fleetd; the service supervisor owns fleetd.sock.
            match std::fs::remove_file(fleet_api::INVENTORY_SOCKET) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            let local = UnixListener::bind(fleet_api::INVENTORY_SOCKET)?;
            let publisher = tokio::spawn(
                Server::builder()
                    .add_service(
                        LocalInventoryServiceServer::new(ipc::Publisher(Arc::clone(&shared)))
                            .max_decoding_message_size(fleet_api::MAX_MESSAGE_BYTES),
                    )
                    .serve_with_incoming(UnixListenerStream::new(local)),
            );
            (
                ipc::Service {
                    manager: None,
                    agent: Some(shared),
                },
                Some(tokio::spawn(agent.run())),
                Some(publisher),
            )
        }
    };
    let result = serve(listener, service).await;
    if let Some(worker) = worker {
        worker.abort();
    }
    if let Some(publisher) = publisher {
        publisher.abort();
    }
    result?;
    Ok(())
}

async fn serve(listener: UnixListener, service: ipc::Service) -> Result<()> {
    Server::builder()
        .add_service(
            FleetServiceServer::new(service)
                .max_decoding_message_size(fleet_api::MAX_MESSAGE_BYTES),
        )
        .serve_with_incoming_shutdown(UnixListenerStream::new(listener), shutdown())
        .await?;
    Ok(())
}
