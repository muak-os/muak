//! Address provisioning helpers for the network supervisor.

#[cfg(any(feature = "ipv4", feature = "ipv6"))]
use alloc::vec::Vec;

use anyhow::Result;
use config::system::network::InterfaceConfig;
#[cfg(feature = "ipv4")]
use config::system::network::Ipv4InterfaceConfig;
#[cfg(feature = "ipv6")]
use config::system::network::Ipv6InterfaceConfig;
use linkctl::interface::Command;
use netdev::Ops;
use netlink::link::ifname::Name;

use super::super::NetworkSupervisor;
use super::{handle, resolve_name, resolve_owner, send_command};

// Applies every configured address to the interface that owns L3.
pub(super) async fn apply_all<N: Ops>(supervisor: &mut NetworkSupervisor<N>) {
    let interfaces = supervisor.config.interfaces.clone();
    for iface_cfg in &interfaces {
        if !has_config(iface_cfg) {
            continue;
        }
        if let Err(e) = apply(supervisor, iface_cfg).await {
            kmsg::warn!("Failed to provision {}: {}", iface_cfg.name, e);
        }
    }
}

// Returns true when the interface declares config for an enabled address family.
pub(in crate::supervisor) const fn has_config(iface_cfg: &InterfaceConfig) -> bool {
    #[cfg(feature = "ipv4")]
    let ipv4 = iface_cfg.ipv4.is_some();
    #[cfg(not(feature = "ipv4"))]
    let ipv4 = false;
    #[cfg(feature = "ipv6")]
    let ipv6 = iface_cfg.ipv6.is_some();
    #[cfg(not(feature = "ipv6"))]
    let ipv6 = false;
    #[cfg(not(any(feature = "ipv4", feature = "ipv6")))]
    let _iface_cfg = iface_cfg;

    ipv4 || ipv6
}

async fn apply<N: Ops>(
    supervisor: &mut NetworkSupervisor<N>,
    iface_cfg: &InterfaceConfig,
) -> Result<()> {
    let name = resolve_name(supervisor, &iface_cfg.name)?;
    let owner = resolve_owner(supervisor, &name);
    supervisor.ops.ensure_up(name.as_str()).await?;

    configure(supervisor, &owner, iface_cfg).await
}

/// Sends the address configuration declared on `iface_cfg` to `owner`.
pub(in crate::supervisor) async fn configure<N: Ops>(
    supervisor: &NetworkSupervisor<N>,
    owner: &Name,
    iface_cfg: &InterfaceConfig,
) -> Result<()> {
    #[cfg(not(feature = "ipv4"))]
    let v4 = None;
    #[cfg(feature = "ipv4")]
    let v4 = iface_cfg
        .ipv4
        .as_ref()
        .and_then(|cfg| v4_config(supervisor, cfg));
    #[cfg(not(feature = "ipv6"))]
    let v6 = None;
    #[cfg(feature = "ipv6")]
    let v6 = iface_cfg
        .ipv6
        .as_ref()
        .and_then(|cfg| v6_config(supervisor, cfg));
    #[cfg(not(any(feature = "ipv4", feature = "ipv6")))]
    let _iface_cfg = iface_cfg;

    if v4.is_none() && v6.is_none() {
        return Ok(());
    }

    let target = handle(supervisor, owner)?;
    send_command(target, Command::Configure { v4, v6 }, owner).await?;
    kmsg::info!("Configuration sent to {}", owner.as_str());

    Ok(())
}

#[cfg(feature = "ipv4")]
fn v4_config<N: Ops>(
    supervisor: &NetworkSupervisor<N>,
    cfg: &Ipv4InterfaceConfig,
) -> Option<addrconf::v4::Config> {
    let dns = supervisor.config.ipv4_dns().collect();
    if cfg.dhcp {
        return Some(addrconf::v4::Config {
            addresses: Vec::new(),
            gateway: None,
            dns,
            dhcp: true,
        });
    }
    if cfg.addresses.is_empty() {
        return None;
    }

    Some(addrconf::v4::Config {
        addresses: cfg
            .addresses
            .iter()
            .map(|cidr| (cidr.address, cidr.prefix))
            .collect(),
        gateway: cfg.gateway,
        dns,
        dhcp: false,
    })
}

#[cfg(feature = "ipv6")]
fn v6_config<N: Ops>(
    supervisor: &NetworkSupervisor<N>,
    cfg: &Ipv6InterfaceConfig,
) -> Option<addrconf::v6::Config> {
    let dns = supervisor.config.ipv6_dns().collect();
    if !cfg.addresses.is_empty() {
        return Some(addrconf::v6::Config {
            addresses: cfg
                .addresses
                .iter()
                .map(|cidr| (cidr.address, cidr.prefix))
                .collect(),
            gateway: cfg.gateway,
            dns,
            autoconf: false,
        });
    }
    if cfg.autoconf && supervisor.config.ipv6 {
        return Some(addrconf::v6::Config {
            addresses: Vec::new(),
            gateway: None,
            dns,
            autoconf: true,
        });
    }

    None
}
