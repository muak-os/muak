//! Linux bridge management for virtual network switching.

use core::future::Future;
use core::net::Ipv4Addr;

use rtnetlink::packet_route::link::BridgeStpState;
use rtnetlink::{Handle, LinkBridge};
use thiserror::Error;

use crate::netlink::Rtnl;
use crate::{address, link, retry, route};

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
    /// Address operation error.
    #[error(transparent)]
    Address(#[from] address::Failure),
    /// Route operation error.
    #[error(transparent)]
    Route(#[from] route::Failure),
    /// Retry operation error.
    #[error(transparent)]
    Retry(#[from] retry::Failure),
}

/// Bridge operation result type.
pub type Result<T> = core::result::Result<T, Failure>;

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
        .map_err(Failure::Create)?;

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
        &format!("bridge '{bridge_name}' creation timeout"),
    )
    .await
    .map_err(Failure::Retry)
}

async fn enslave_interface_to_bridge(
    handle: &Handle,
    phys_index: u32,
    br_index: u32,
    physical_iface: &str,
    bridge_name: &str,
) -> Result<()> {
    let already_enslaved = match link::master_index(handle, phys_index).await {
        Ok(Some(master)) => master == br_index,
        _ => false,
    };

    if already_enslaved {
        let _result = link::bring_up(handle, phys_index).await;
        return Ok(());
    }

    let _result = link::bring_down(handle, phys_index).await;

    retry::run(
        || async { link::set_master(handle, phys_index, br_index).await },
        ENSLAVE_RETRIES,
        ENSLAVE_RETRY_DELAY_MS,
        &format!("failed to enslave {physical_iface} to {bridge_name}"),
    )
    .await
    .map_err(Failure::Retry)?;

    let _result = link::bring_up(handle, phys_index).await;

    println!("Enslaved {physical_iface} to bridge {bridge_name}");

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
            println!("Restored default route via {gw}");
        }

        println!("Transferred IP {ip}/{prefix} to bridge {bridge_name}");
    }

    Ok(())
}

/// Trait covering bridge netlink operations.
pub trait Ops: Clone + Send + Sync + 'static {
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

impl Ops for Rtnl {
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
        println!("Attaching {iface_name} to bridge {bridge_name}");
        let iface_index = link::get_index(&self.handle, iface_name).await?;
        let bridge_index = link::get_index(&self.handle, bridge_name).await?;
        link::set_master(&self.handle, iface_index, bridge_index).await?;
        println!("{iface_name} attached to bridge {bridge_name}");
        Ok(())
    }
}
