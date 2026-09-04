//! DHCP lease life cycle management for a per-interface actor.

use core::net::Ipv4Addr;
use core::pin::Pin;
use std::time::SystemTime;

use anyhow::Result;
use netlib::address::IpConfig;
use netlib::netlink::Ops;
use tokio::time::{Sleep, sleep};

use super::Actor;
use crate::dhcp::client::DhcpConnector;
use crate::dhcp::codec::DhcpNak;
use crate::dhcp::manager::Manager;
use crate::dhcp::{self, Lease, State};
use crate::interface::commands::ApplyMode;
use crate::interface::state::Lifecycle;
use crate::statemachine::StateMachine as _;

/// Holds the three timers handles that drive the DHCP renewal state machine.
pub(super) struct LeaseTimers {
    pub renew: Option<Pin<Box<Sleep>>>,
    pub rebind: Option<Pin<Box<Sleep>>>,
    pub expire: Option<Pin<Box<Sleep>>>,
}

impl LeaseTimers {
    /// Returns a new instance with all timers disarmed.
    pub fn new() -> Self {
        Self {
            renew: None,
            rebind: None,
            expire: None,
        }
    }

    /// Arms all three timers from the deadlines encoded in `lease`.
    pub fn arm(&mut self, lease: &Lease) {
        self.disarm();
        let now = SystemTime::now();
        self.renew = deadline_to_sleep(
            now,
            lease
                .obtained_at
                .checked_add(lease.renewal_time)
                .unwrap_or(now),
        );
        self.rebind = deadline_to_sleep(
            now,
            lease
                .obtained_at
                .checked_add(lease.rebind_time)
                .unwrap_or(now),
        );
        self.expire = deadline_to_sleep(now, lease.expiry());
    }

    /// Cancels all active timers.
    pub fn disarm(&mut self) {
        self.renew = None;
        self.rebind = None;
        self.expire = None;
    }
}

/// Applies DHCP configuration in the selected mode.
pub(super) async fn apply<N: Ops, C: DhcpConnector>(
    actor: &mut Actor<N>,
    mode: ApplyMode,
    connector: &C,
) {
    match mode {
        ApplyMode::Provision => start(actor, connector).await,
        ApplyMode::Reconcile => reconcile(actor, connector).await,
    }
}

/// Initialises a `Manager` (binding the socket) and marks the interface as configuring.
pub(super) async fn start<N: Ops, C: DhcpConnector>(actor: &mut Actor<N>, connector: &C) {
    actor.set_state(Lifecycle::Configuring);
    let mac = actor.snapshot.mac;
    match Manager::new(actor.snapshot.name.as_str(), mac, connector).await {
        Ok(mgr) => actor.dhcp = Some(mgr),
        Err(e) => {
            kmsg::warn!(
                "Failed to create DHCP socket on {}: {}",
                actor.snapshot.name,
                e
            );
            actor.set_state(Lifecycle::Failed);
        }
    }
}

/// Re-applies DHCP state or restarts acquisition when no lease is cached.
pub(super) async fn reconcile<N: Ops, C: DhcpConnector>(actor: &mut Actor<N>, connector: &C) {
    if let Some(lease) = actor.snapshot.lease.clone() {
        let index = actor.snapshot.index;
        if let Err(e) = apply_lease(actor, index, &lease).await {
            kmsg::warn!("DHCP reconcile failed on {}: {}", actor.snapshot.name, e);
            return;
        }

        store_lease(actor, &lease);
        transition(actor, State::Bound);
        actor.timers.arm(&lease);
        let _configured = actor.ensure_configured_state();
        return;
    }

    if actor.dhcp.is_none() {
        start(actor, connector).await;
    }
}

/// Applies a freshly acquired DHCP lease and clears the in-progress manager.
pub(super) async fn acquired<N: Ops>(actor: &mut Actor<N>, lease: Lease) {
    actor.dhcp = None;
    let index = actor.snapshot.index;
    if let Err(e) = commit_lease(actor, index, lease).await {
        kmsg::warn!(
            "Failed to apply DHCP lease on {}: {}",
            actor.snapshot.name,
            e
        );
        actor.set_state(Lifecycle::Failed);
        return;
    }
    actor.set_state(Lifecycle::Configured);
    if let Some(lease) = actor.snapshot.lease.as_ref() {
        kmsg::info!(
            "DHCP acquired on {}: {}",
            actor.snapshot.name,
            lease.assigned_ip
        );
    }
}

/// Re-applies an existing lease after a link-up event without a new DORA exchange.
pub(super) async fn recover_with_lease<N: Ops>(actor: &mut Actor<N>, lease: Lease) {
    let index = actor.snapshot.index;
    if let Err(e) = apply_lease(actor, index, &lease).await {
        kmsg::warn!(
            "Failed to re-apply lease on link-up for {}: {}",
            actor.snapshot.name,
            e
        );
        actor.set_state(Lifecycle::Failed);
        return;
    }
    actor.timers.arm(&lease);
    actor.set_state(Lifecycle::Configured);
}

pub(super) async fn renew_lease<N: Ops, C: DhcpConnector>(actor: &mut Actor<N>, connector: &C) {
    kmsg::info!("DHCP RENEW for {}", actor.snapshot.name);
    transition(actor, State::Renewing);
    if let Err(e) = do_renew(actor, connector).await {
        kmsg::warn!("DHCP RENEW failed for {}: {}", actor.snapshot.name, e);
    }
}

pub(super) async fn rebind_lease<N: Ops, C: DhcpConnector>(actor: &mut Actor<N>, connector: &C) {
    kmsg::info!("DHCP REBIND for {}", actor.snapshot.name);
    transition(actor, State::Rebinding);
    if let Err(e) = do_rebind(actor, connector).await {
        kmsg::warn!("DHCP REBIND failed for {}: {}", actor.snapshot.name, e);
    }
}

/// Re-runs a full DORA exchange to recover from a NAK or lease expiry.
pub(super) async fn do_full_dora<N: Ops, C: DhcpConnector>(
    actor: &mut Actor<N>,
    connector: &C,
) -> Result<()> {
    kmsg::warn!("DHCP re-acquiring on {}", actor.snapshot.name);
    transition(actor, State::Init);
    actor.set_state(Lifecycle::Configuring);
    actor.timers.disarm();

    let mac = actor.snapshot.mac;
    let socket = connector.create_raw(actor.snapshot.name.as_str()).await?;
    let lease = dhcp::client::run(&socket, &mac).await.inspect_err(|_| {
        actor.set_state(Lifecycle::Failed);
    })?;

    let index = actor.snapshot.index;
    commit_lease(actor, index, lease).await?;
    actor.set_state(Lifecycle::Configured);

    if let Some(lease) = actor.snapshot.lease.as_ref() {
        kmsg::info!(
            "DHCP re-acquired on {}: {}",
            actor.snapshot.name,
            lease.assigned_ip
        );
    }

    Ok(())
}

/// Applies the network-level changes from a lease.
pub(super) async fn apply_lease<N: Ops>(
    actor: &mut Actor<N>,
    index: u32,
    lease: &Lease,
) -> Result<()> {
    actor
        .ops
        .ensure_ipv4(index, lease.assigned_ip, lease.prefix_len)
        .await?;

    if let Some(gw) = lease.gateway {
        actor.ops.ensure_default_route(gw).await?;
    } else {
        kmsg::info!(
            "No gateway in DHCP lease on {}, skipping default route",
            actor.snapshot.name
        );
    }

    Ok(())
}

/// Persists the lease into the snapshot and publishes it.
pub(super) fn store_lease<N: Ops>(actor: &mut Actor<N>, lease: &Lease) {
    actor.snapshot.ip = Some(IpConfig {
        address: lease.assigned_ip,
        prefix_len: lease.prefix_len,
        gateway: lease.gateway,
        dns: lease.dns_servers.clone(),
    });
    actor.snapshot.lease = Some(lease.clone());
    actor.publish_snapshot();
}

pub(super) fn transition<N: Ops>(actor: &mut Actor<N>, next: State) {
    let Some(current) = actor.snapshot.dhcp_state.as_mut() else {
        actor.snapshot.dhcp_state = Some(next);
        return;
    };
    if let Err(e) = current.transition(next) {
        kmsg::warn!(
            "DHCP state transition rejected on {}: {}",
            actor.snapshot.name,
            e
        );
    }
}

async fn do_renew<N: Ops, C: DhcpConnector>(actor: &mut Actor<N>, connector: &C) -> Result<()> {
    let (mac, server_ip, assigned_ip) = lease_params(actor)?;
    let socket = connector
        .create_unicast(actor.snapshot.name.as_str(), assigned_ip)
        .await?;
    let result = dhcp::client::renew(&socket, &mac, server_ip, assigned_ip).await;
    match result {
        Ok(lease) => apply_renewed_lease(actor, &lease).await,
        Err(e) if e.downcast_ref::<DhcpNak>().is_some() => {
            kmsg::warn!(
                "DHCP RENEW NAK for {}, returning to INIT",
                actor.snapshot.name
            );
            do_full_dora(actor, connector).await
        }
        Err(e) => Err(e),
    }
}

async fn do_rebind<N: Ops, C: DhcpConnector>(actor: &mut Actor<N>, connector: &C) -> Result<()> {
    let (mac, server_ip, assigned_ip) = lease_params(actor)?;
    let socket = connector.create_raw(actor.snapshot.name.as_str()).await?;
    let result = dhcp::client::rebind(&socket, &mac, server_ip, assigned_ip).await;
    match result {
        Ok(lease) => apply_renewed_lease(actor, &lease).await,
        Err(e) if e.downcast_ref::<DhcpNak>().is_some() => {
            kmsg::warn!(
                "DHCP REBIND NAK for {}, returning to INIT",
                actor.snapshot.name
            );
            do_full_dora(actor, connector).await
        }
        Err(e) => Err(e),
    }
}

/// Applies a renewed lease without changing the interface state.
async fn apply_renewed_lease<N: Ops>(actor: &mut Actor<N>, lease: &Lease) -> Result<()> {
    let index = actor.snapshot.index;
    commit_lease(actor, index, lease.clone()).await?;
    kmsg::info!("DHCP lease renewed for {}", actor.snapshot.name);

    Ok(())
}

/// Applies kernel-level changes, stores the lease, advances DHCP state, and arms timers.
async fn commit_lease<N: Ops>(actor: &mut Actor<N>, index: u32, lease: Lease) -> Result<()> {
    apply_lease(actor, index, &lease).await?;
    store_lease(actor, &lease);
    transition(actor, State::Bound);
    actor.timers.arm(&lease);

    Ok(())
}

fn lease_params<N: Ops>(actor: &Actor<N>) -> Result<([u8; 6], Ipv4Addr, Ipv4Addr)> {
    let lease = actor
        .snapshot
        .lease
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no DHCP lease on {}", actor.snapshot.name))?;

    Ok((actor.snapshot.mac, lease.server_ip, lease.assigned_ip))
}

fn deadline_to_sleep(now: SystemTime, deadline: SystemTime) -> Option<Pin<Box<Sleep>>> {
    let dur = deadline.duration_since(now).ok()?;

    Some(Box::pin(sleep(dur)))
}
