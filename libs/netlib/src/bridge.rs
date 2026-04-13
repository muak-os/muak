//! Linux bridge management for virtual network switching.

use std::future::Future;
use std::net::Ipv4Addr;

use rtnetlink::packet_route::link::BridgeStpState;
use rtnetlink::{Handle, LinkBridge};
use thiserror::Error;

use crate::ops::RtnetlinkOps;
use crate::{address, link, retry, route};

/// Number of attempts to check for bridge creation before giving up.
const CREATE_RETRIES: u8 = 30;

/// Delay between attempts to check for bridge creation.
const CREATE_RETRY_DELAY_MS: u64 = 100;

/// Number of attempts to enslave an interface to the bridge before giving up.
const ENSLAVE_RETRIES: u8 = 5;

/// Delay between attempts to enslave an interface to the bridge.
const ENSLAVE_RETRY_DELAY_MS: u64 = 100;

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to create bridge: {0}")]
    Create(#[source] rtnetlink::Error),
    #[error(transparent)]
    Link(#[from] link::Error),
    #[error(transparent)]
    Address(#[from] address::Error),
    #[error(transparent)]
    Route(#[from] route::Error),
    #[error(transparent)]
    Retry(#[from] retry::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

async fn create_or_reconfigure_bridge(
    handle: &Handle,
    bridge_name: &str,
    stp: bool,
) -> Result<u32> {
    if link::exists(handle, bridge_name).await? {
        let index = link::get_index(handle, bridge_name).await?;
        link::bring_up(handle, index).await?;
        return Ok(index);
    }

    create_bridge(handle, bridge_name, stp).await
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
        .map_err(Error::Create)?;

    retry::wait_for_condition(
        || async {
            if link::exists(handle, bridge_name).await.ok()? {
                let index = link::get_index(handle, bridge_name).await.ok()?;
                link::bring_up(handle, index).await.ok()?;
                Some(index)
            } else {
                None
            }
        },
        CREATE_RETRIES,
        CREATE_RETRY_DELAY_MS,
        &format!("bridge '{}' creation timeout", bridge_name),
    )
    .await
    .map_err(Error::Retry)
}

async fn enslave_interface_to_bridge(
    handle: &Handle,
    phys_index: u32,
    br_index: u32,
    physical_iface: &str,
    bridge_name: &str,
) -> Result<()> {
    link::bring_down(handle, phys_index).await.ok();

    retry::run(
        || async { link::set_master(handle, phys_index, br_index).await },
        ENSLAVE_RETRIES,
        ENSLAVE_RETRY_DELAY_MS,
        &format!("failed to enslave {} to {}", physical_iface, bridge_name),
    )
    .await
    .map_err(Error::Retry)?;

    link::bring_up(handle, phys_index).await.ok();

    println!("Enslaved {} to bridge {}", physical_iface, bridge_name);

    Ok(())
}

async fn transfer_ip_to_bridge(
    handle: &Handle,
    phys_index: u32,
    br_index: u32,
    bridge_name: &str,
    gateway: Option<Ipv4Addr>,
) -> Result<()> {
    let phys_ip = address::find_ipv4(handle, phys_index).await?;
    let has_bridge_ip = address::has_ipv4(handle, br_index).await?;

    if let Some((ip, prefix)) = phys_ip
        && !has_bridge_ip
    {
        address::remove_ipv4(handle, phys_index, ip).await?;
        address::add_ipv4(handle, br_index, ip, prefix).await?;

        if let Some(gw) = gateway {
            route::add_default_route(handle, gw).await?;
            println!("Restored default route via {}", gw);
        }

        println!("Transferred IP {}/{} to bridge {}", ip, prefix, bridge_name);
    }

    Ok(())
}

/// Trait covering bridge netlink operations.
pub trait BridgeOps: Clone + Send + Sync + 'static {
    /// Creates a bridge with the given configuration and attaches a physical interface.
    fn ensure_bridge(
        &self,
        bridge_name: &str,
        physical_iface: &str,
        gateway: Option<Ipv4Addr>,
        stp: bool,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Attaches a named interface to a named bridge.
    fn attach_to_bridge(
        &self,
        iface_name: &str,
        bridge_name: &str,
    ) -> impl Future<Output = Result<()>> + Send;
}

impl BridgeOps for RtnetlinkOps {
    async fn ensure_bridge(
        &self,
        bridge_name: &str,
        physical_iface: &str,
        gateway: Option<Ipv4Addr>,
        stp: bool,
    ) -> Result<()> {
        let phys_index = link::get_index(&self.handle, physical_iface).await?;
        let br_index = create_or_reconfigure_bridge(&self.handle, bridge_name, stp).await?;
        enslave_interface_to_bridge(
            &self.handle,
            phys_index,
            br_index,
            physical_iface,
            bridge_name,
        )
        .await?;
        transfer_ip_to_bridge(&self.handle, phys_index, br_index, bridge_name, gateway).await
    }

    async fn attach_to_bridge(&self, iface_name: &str, bridge_name: &str) -> Result<()> {
        println!("Attaching {} to bridge {}", iface_name, bridge_name);
        let iface_index = link::get_index(&self.handle, iface_name).await?;
        let bridge_index = link::get_index(&self.handle, bridge_name).await?;
        link::set_master(&self.handle, iface_index, bridge_index).await?;
        println!("{} attached to bridge {}", iface_name, bridge_name);
        Ok(())
    }
}
