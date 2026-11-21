//! Address provisioning helpers for the network supervisor.

use alloc::vec::Vec;

use anyhow::Result;
use config::system::network::InterfaceConfig;
#[cfg(feature = "ipv4")]
use config::system::network::Ipv4InterfaceConfig;
#[cfg(feature = "ipv6")]
use config::system::network::Ipv6InterfaceConfig;
use netdev::Ops;
use netlink::link::ifname::Name;
use netstack::interface::Command;
use netstack::provision;

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
    #[cfg(feature = "ipv4")]
    let v4 = iface_cfg
        .ipv4
        .as_ref()
        .and_then(|cfg| v4_config(supervisor, cfg));
    #[cfg(feature = "ipv6")]
    let v6 = iface_cfg
        .ipv6
        .as_ref()
        .and_then(|cfg| v6_config(supervisor, cfg));

    #[cfg(all(feature = "ipv4", feature = "ipv6"))]
    if v4.is_none() && v6.is_none() {
        return Ok(());
    }
    #[cfg(all(feature = "ipv4", not(feature = "ipv6")))]
    if v4.is_none() {
        return Ok(());
    }
    #[cfg(all(feature = "ipv6", not(feature = "ipv4")))]
    if v6.is_none() {
        return Ok(());
    }

    let target = handle(supervisor, owner)?;
    send_command(
        target,
        Command::Configure {
            #[cfg(feature = "ipv4")]
            v4,
            #[cfg(feature = "ipv6")]
            v6,
        },
        owner,
    )
    .await
}

#[cfg(feature = "ipv4")]
fn v4_config<N: Ops>(
    supervisor: &NetworkSupervisor<N>,
    cfg: &Ipv4InterfaceConfig,
) -> Option<provision::v4::Config> {
    let dns = supervisor.config.ipv4_dns().collect();
    if cfg.dhcp {
        return Some(provision::v4::Config {
            addresses: Vec::new(),
            gateway: None,
            dns,
            dhcp: true,
        });
    }
    if cfg.addresses.is_empty() {
        return None;
    }

    Some(provision::v4::Config {
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
) -> Option<provision::v6::Config> {
    let dns = supervisor.config.ipv6_dns().collect();
    if !cfg.addresses.is_empty() {
        return Some(provision::v6::Config {
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
        return Some(provision::v6::Config {
            addresses: Vec::new(),
            gateway: None,
            dns,
            autoconf: true,
        });
    }

    None
}
