//! Optional, read-only inventory publication by the workload owner.

use core::time::Duration;
use std::path::PathBuf;

use anyhow::Result;
use fleet_api::fleet::local_inventory_service_client::LocalInventoryServiceClient;
use fleet_api::fleet::{NodeSnapshot, VmSummary};
use fleet_api::unix::Connector;
use granola::runtime::signal::shutdown;
use tonic::transport::Endpoint;

use crate::actor::VmActorHandle;

pub(crate) async fn publish(handle: VmActorHandle, hostname: String, interval: u64) {
    loop {
        if let Err(error) = send(&handle, &hostname).await {
            kmsg::warn!("Fleet inventory publication unavailable: {error}");
        }
        tokio::select! {
            () = shutdown() => break,
            () = tokio::time::sleep(Duration::from_secs(interval)) => {},
        }
    }
}

async fn send(handle: &VmActorHandle, hostname: &str) -> Result<()> {
    let vms = handle
        .list()
        .await?
        .into_iter()
        .map(|vm| VmSummary {
            vm_id: vm.vm_id,
            name: vm.config.map_or_else(String::new, |config| config.name),
            state: vm.state,
        })
        .collect();
    let channel = Endpoint::from_static("http://localhost")
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(5))
        .connect_with_connector(Connector(PathBuf::from(fleet_api::INVENTORY_SOCKET)))
        .await?;
    LocalInventoryServiceClient::new(channel)
        .publish(NodeSnapshot {
            hostname: hostname.to_owned(),
            vms,
        })
        .await?;
    Ok(())
}
