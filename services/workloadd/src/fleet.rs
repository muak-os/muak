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
use crate::proto::vm::VmInfo;

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
    let vms = handle.list().await?.into_iter().map(summary).collect();
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

fn summary(vm: VmInfo) -> VmSummary {
    let name = vm.config.map_or_else(String::new, |config| config.name);
    VmSummary {
        name: display_name(&name, &vm.vm_id),
        vm_id: vm.vm_id,
        state: vm.state,
    }
}

fn display_name(name: &str, id: &str) -> String {
    if name.is_empty() {
        return id.to_owned();
    }
    if name.len() <= 255 {
        return name.to_owned();
    }
    let mut result = String::new();
    for character in name.chars() {
        if result.len().saturating_add(character.len_utf8()) > 252 {
            break;
        }
        result.push(character);
    }
    result.push_str("...");
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::vm::VmConfig;

    #[test]
    fn valid_local_vm_names_cannot_block_the_inventory_projection() {
        // ARRANGE: workloadd accepts empty and long names through its local API.
        let id = "f0b96bb9-54ee-4a9f-abab-186e2178096a";
        let unnamed = VmInfo {
            vm_id: id.to_owned(),
            state: 3,
            config: None,
            ..VmInfo::default()
        };
        let long = VmInfo {
            vm_id: id.to_owned(),
            state: 5,
            config: Some(VmConfig {
                name: "é".repeat(200),
                ..VmConfig::default()
            }),
            ..VmInfo::default()
        };
        // ACT
        let unnamed = summary(unnamed);
        let long = summary(long);
        // ASSERT: only display labels change; identity and state remain exact.
        assert_eq!(unnamed.name, id);
        assert_eq!(unnamed.state, 3_i32);
        assert_eq!(long.vm_id, id);
        assert_eq!(long.state, 5_i32);
        assert_eq!(long.name.len(), 255);
        assert!(long.name.ends_with("..."));
    }
}
