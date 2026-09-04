//! SLAAC event handling for a per-interface actor.

use alloc::sync::Arc;
use core::net::Ipv6Addr;

use netlib::address::Ipv6Config;
use netlib::netlink::Ops;

use super::Actor;
use crate::interface::commands::ApplyMode;
use crate::slaac::manager::{Manager as SlaacManager, SlaacEvent};

/// Applies SLAAC configuration in the selected mode.
pub(super) async fn apply<N: Ops>(actor: &mut Actor<N>, mode: ApplyMode) {
    match mode {
        ApplyMode::Provision => start(actor).await,
        ApplyMode::Reconcile => reconcile(actor).await,
    }
}

/// Initialises a `SlaacManager`.
pub(super) async fn start<N: Ops>(actor: &mut Actor<N>) {
    let iface = actor.snapshot.name.to_string();
    let mac = actor.snapshot.mac;
    match SlaacManager::new(iface, mac, Arc::clone(&actor.config)).await {
        Ok(mgr) => {
            kmsg::info!("Starting SLAAC on {}", actor.snapshot.name);
            actor.slaac = Some(mgr);
        }
        Err(e) => {
            kmsg::info!(
                "SLAAC unavailable on {}: {} (continuing with IPv4)",
                actor.snapshot.name,
                e
            );
        }
    }
}

/// Reapplies the current IPv6 snapshot and restarts SLAAC if needed.
pub(super) async fn reconcile<N: Ops>(actor: &mut Actor<N>) {
    if let Some(ipv6) = actor.snapshot.ipv6.clone() {
        let index = actor.snapshot.index;
        let applied = apply_ipv6_configuration(actor, index, &ipv6)
            .await
            .inspect_err(|e| {
                kmsg::warn!("SLAAC reconcile failed on {}: {}", actor.snapshot.name, e);
            });
        if applied.is_ok() && !actor.ensure_configured_state() {
            actor.publish_snapshot();
        }
    }

    if actor.slaac.is_none() {
        start(actor).await;
    }
}

pub(super) async fn handle_event<N: Ops>(actor: &mut Actor<N>, event: SlaacEvent) {
    match event {
        SlaacEvent::Configured {
            address,
            prefix_len,
            gateway,
            dns,
        } => {
            on_configured(actor, address, prefix_len, gateway, dns).await;
        }
        SlaacEvent::AddressDeprecated { address } => {
            kmsg::info!("IPv6 address deprecated: {}", address);
        }
        SlaacEvent::AddressExpired { address } => {
            on_address_expired(actor, address).await;
        }
        SlaacEvent::RouterExpired { router } => {
            on_router_expired(actor, router).await;
        }
        SlaacEvent::DnsUpdated { servers } => {
            on_dns_updated(actor, servers);
        }
        SlaacEvent::Failed { reason } => {
            kmsg::warn!("SLAAC failed: {} (continuing with IPv4)", reason);
            actor.slaac = None;
        }
    }
}

async fn apply_ipv6_configuration<N: Ops>(
    actor: &mut Actor<N>,
    index: u32,
    ipv6: &Ipv6Config,
) -> anyhow::Result<()> {
    actor
        .ops
        .ensure_ipv6(index, ipv6.address, ipv6.prefix_len)
        .await?;

    if let Some(gateway) = ipv6.gateway {
        kmsg::info!("Setting IPv6 default route via {}", gateway);
        actor.ops.ensure_default_route_v6(gateway).await?;
    }

    Ok(())
}

async fn on_configured<N: Ops>(
    actor: &mut Actor<N>,
    address: Ipv6Addr,
    prefix_len: u8,
    gateway: Ipv6Addr,
    dns: Vec<Ipv6Addr>,
) {
    kmsg::info!("SLAAC configured: {} via {}", address, gateway);

    let index = actor.snapshot.index;
    let ipv6 = Ipv6Config {
        address,
        prefix_len,
        gateway: Some(gateway),
        dns,
    };

    if let Err(e) = apply_ipv6_configuration(actor, index, &ipv6).await {
        kmsg::warn!("Failed to apply IPv6 configuration: {}", e);
        return;
    }

    actor.snapshot.ipv6 = Some(ipv6);
    if !actor.ensure_configured_state() {
        actor.publish_snapshot();
    }
}

async fn on_address_expired<N: Ops>(actor: &mut Actor<N>, address: Ipv6Addr) {
    kmsg::info!("IPv6 address expired: {}", address);

    let index = actor.snapshot.index;
    if let Err(e) = actor.ops.remove_ipv6(index, address).await {
        kmsg::warn!("Failed to remove expired IPv6 address: {}", e);
    }
    actor.snapshot.ipv6 = None;
    actor.publish_snapshot();
}

async fn on_router_expired<N: Ops>(actor: &mut Actor<N>, router: Ipv6Addr) {
    kmsg::info!("IPv6 router expired: {}", router);
    if let Err(e) = actor.ops.remove_default_route_v6(router).await {
        kmsg::warn!("Failed to remove IPv6 default route: {}", e);
    }
}

fn on_dns_updated<N: Ops>(actor: &mut Actor<N>, servers: Vec<Ipv6Addr>) {
    kmsg::info!("IPv6 DNS updated: {} servers", servers.len());

    if let Some(ipv6) = actor.snapshot.ipv6.as_mut() {
        ipv6.dns = servers;
        actor.publish_snapshot();
    }
}
