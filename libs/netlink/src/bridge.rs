//! Linux bridge management for virtual network switching.

use core::future::Future;

use rtnetlink::packet_route::link::BridgeStpState;
use rtnetlink::{Handle, LinkBridge};
use thiserror::Error;

use crate::link::Ops as _;
use crate::link::device::Device;
use crate::link::state::State;
use crate::{Rtnl, link, retry};

/// Number of attempts to check for bridge creation before giving up.
const CREATE_RETRIES: u8 = 30;

/// Delay between attempts to check for bridge creation.
const CREATE_RETRY_DELAY_MS: u64 = 100;

/// Number of attempts to enslave an interface to the bridge before giving up.
const ENSLAVE_RETRIES: u8 = 5;

/// Delay between attempts to enslave an interface to the bridge.
const ENSLAVE_RETRY_DELAY_MS: u64 = 100;

/// Bridge operation failures.
#[derive(Debug, Error)]
pub enum Failure {
    /// Failed to create bridge.
    #[error("failed to create bridge: {0}")]
    Create(#[source] rtnetlink::Error),
    /// Link operation error.
    #[error(transparent)]
    Link(#[from] link::Failure),
    /// Retry operation error.
    #[error(transparent)]
    Retry(#[from] retry::Failure),
}

/// Bridge operation result type.
pub type Result<T> = core::result::Result<T, Failure>;

/// Identity and link state of a bridge after it has been ensured.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bridge {
    /// Hardware address the bridge is pinned to.
    pub mac: [u8; 6],
    /// Link state derived from the attached port's carrier.
    pub link: State,
}

/// Trait covering bridge netlink operations.
pub trait Ops: Clone + Send + Sync + 'static {
    /// Ensures the bridge exists with the given port attached and returns its identity and link state.
    fn ensure_bridge(
        &self,
        bridge_name: &str,
        port_name: &str,
        stp: bool,
    ) -> impl Future<Output = Result<Bridge>> + Send;
}

impl Ops for Rtnl {
    async fn ensure_bridge(&self, bridge_name: &str, port_name: &str, stp: bool) -> Result<Bridge> {
        let devices = self.devices().await?;
        let port = devices
            .iter()
            .find(|device| device.name == port_name)
            .ok_or_else(|| Failure::Link(link::Failure::NotFound(port_name.to_owned())))?;

        if let Some(bridge) = devices.iter().find(|device| device.name == bridge_name)
            && is_attached(port, bridge, &devices)
            && bridge.admin_up
            && port.admin_up
        {
            return Ok(Bridge {
                mac: bridge.mac,
                link: carrier_state(port.carrier),
            });
        }

        let (br_index, created) =
            create_or_reconfigure_bridge(&self.handle, bridge_name, stp).await?;

        let mac = devices
            .iter()
            .find(|device| device.name == bridge_name)
            .map_or(port.mac, |device| device.mac);
        enslave_interface_to_bridge(&self.handle, port.index, br_index, port_name, bridge_name)
            .await?;

        for device in devices
            .iter()
            .filter(|device| device.master == Some(br_index) && device.index != port.index)
        {
            link::handle::detach(&self.handle, device.index).await?;
        }

        if created {
            link::handle::set_address(&self.handle, br_index, mac).await?;
        }

        let link = carrier_state(port.carrier);

        Ok(Bridge { mac, link })
    }
}

fn is_attached(port: &Device, bridge: &Device, devices: &[Device]) -> bool {
    port.master == Some(bridge.index)
        && !devices
            .iter()
            .any(|device| device.master == Some(bridge.index) && device.index != port.index)
}

fn carrier_state(carrier: bool) -> State {
    if carrier { State::Up } else { State::NoCarrier }
}

async fn create_or_reconfigure_bridge(
    handle: &Handle,
    bridge_name: &str,
    stp: bool,
) -> Result<(u32, bool)> {
    if link::handle::exists(handle, bridge_name).await? {
        let index = link::handle::get_index(handle, bridge_name).await?;
        link::handle::bring_up(handle, index).await?;
        return Ok((index, false));
    }

    create_bridge(handle, bridge_name, stp)
        .await
        .map(|index| (index, true))
}

async fn create_bridge(handle: &Handle, bridge_name: &str, stp: bool) -> Result<u32> {
    let stp_state = if stp {
        BridgeStpState::KernelStp
    } else {
        BridgeStpState::Disabled
    };

    handle
        .link()
        .add(LinkBridge::new(bridge_name).stp_state(stp_state).build())
        .execute()
        .await
        .map_err(Failure::Create)?;

    retry::wait_for_condition(
        || async {
            if link::handle::exists(handle, bridge_name).await.ok()? {
                let index = link::handle::get_index(handle, bridge_name).await.ok()?;
                link::handle::bring_up(handle, index).await.ok()?;
                Some(index)
            } else {
                None
            }
        },
        CREATE_RETRIES,
        CREATE_RETRY_DELAY_MS,
        &format!("bridge '{bridge_name}' creation timeout"),
    )
    .await
    .map_err(Failure::Retry)
}

async fn enslave_interface_to_bridge(
    handle: &Handle,
    phys_index: u32,
    br_index: u32,
    port_name: &str,
    bridge_name: &str,
) -> Result<()> {
    let already_enslaved = match link::handle::master_index(handle, phys_index).await {
        Ok(Some(master)) => master == br_index,
        _ => false,
    };

    if already_enslaved {
        let _result = link::handle::bring_up(handle, phys_index).await;
        return Ok(());
    }

    let _result = link::handle::bring_down(handle, phys_index).await;

    retry::run(
        || async { link::handle::set_master(handle, phys_index, br_index).await },
        ENSLAVE_RETRIES,
        ENSLAVE_RETRY_DELAY_MS,
        &format!("failed to enslave {port_name} to {bridge_name}"),
    )
    .await
    .map_err(Failure::Retry)?;

    let _result = link::handle::bring_up(handle, phys_index).await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::is_attached;
    use crate::link::device::Device;

    fn device(index: u32, name: &str, master: Option<u32>) -> Device {
        Device {
            index,
            name: name.to_owned(),
            mac: [0; 6],
            is_virtual: false,
            admin_up: true,
            carrier: true,
            master,
        }
    }

    #[test]
    fn is_attached_when_port_is_the_only_member() {
        // ARRANGE
        let port = device(2, "eth0", Some(1));
        let bridge = device(1, "br0", None);
        let devices = vec![bridge.clone(), port.clone()];

        // ACT
        let attached = is_attached(&port, &bridge, &devices);

        // ASSERT
        assert!(attached);
    }

    #[test]
    fn is_attached_rejects_extra_member() {
        // ARRANGE
        let port = device(2, "eth0", Some(1));
        let bridge = device(1, "br0", None);
        let stray = device(3, "eth1", Some(1));
        let devices = vec![bridge.clone(), port.clone(), stray];

        // ACT
        let attached = is_attached(&port, &bridge, &devices);

        // ASSERT
        assert!(!attached);
    }

    #[test]
    fn is_attached_rejects_different_master() {
        // ARRANGE
        let port = device(2, "eth0", Some(4));
        let bridge = device(1, "br0", None);
        let devices = vec![bridge.clone(), port.clone(), device(3, "br1", None)];

        // ACT
        let attached = is_attached(&port, &bridge, &devices);

        // ASSERT
        assert!(!attached);
    }

    #[test]
    fn is_attached_rejects_unattached_port() {
        // ARRANGE
        let port = device(2, "eth0", None);
        let bridge = device(1, "br0", None);
        let devices = vec![bridge.clone(), port.clone()];

        // ACT
        let attached = is_attached(&port, &bridge, &devices);

        // ASSERT
        assert!(!attached);
    }
}
