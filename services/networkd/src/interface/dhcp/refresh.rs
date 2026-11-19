//! DHCP renew and rebind exchanges with NAK recovery.

use anyhow::Result;
use dhcp::Scratch;
use dhcp::connector::Connector;
use dhcp::lease::Lease;
use netlib::netlink::Ops;

use super::{Actor, commit, do_full_dora};
use crate::interface::lifecycle::DhcpState;

pub(in crate::interface) async fn renew<N: Ops, C: Connector>(actor: &mut Actor<N>, connector: &C) {
    kmsg::info!("DHCP RENEW for {}", actor.snapshot.name);
    commit::transition(actor, DhcpState::Renewing);
    if let Err(e) = attempt_renew(actor, connector).await {
        kmsg::warn!("DHCP RENEW failed for {}: {}", actor.snapshot.name, e);
    }
}

pub(in crate::interface) async fn rebind<N: Ops, C: Connector>(
    actor: &mut Actor<N>,
    connector: &C,
) {
    kmsg::info!("DHCP REBIND for {}", actor.snapshot.name);
    commit::transition(actor, DhcpState::Rebinding);
    if let Err(e) = attempt_rebind(actor, connector).await {
        kmsg::warn!("DHCP REBIND failed for {}: {}", actor.snapshot.name, e);
    }
}

async fn attempt_renew<N: Ops, C: Connector>(actor: &mut Actor<N>, connector: &C) -> Result<()> {
    let (mac, server_ip, assigned_ip) = commit::params(actor)?;
    let socket = connector
        .create_unicast(actor.snapshot.name.as_str(), assigned_ip)
        .await?;
    let mut msg = [0_u8; dhcp::MAX_MESSAGE_LEN];
    let mut frame = [0_u8; dhcp::MAX_FRAME_LEN];
    match dhcp::exchange::renew(
        &socket,
        mac,
        server_ip,
        assigned_ip,
        Scratch::new(&mut msg, &mut frame),
    )
    .await
    {
        Ok(lease) => apply_renewed_lease(actor, &lease).await,
        Err(dhcp::Failure::Nak) => {
            kmsg::warn!(
                "DHCP RENEW NAK for {}, returning to INIT",
                actor.snapshot.name
            );
            do_full_dora(actor, connector).await
        }
        Err(error) => Err(error.into()),
    }
}

async fn attempt_rebind<N: Ops, C: Connector>(actor: &mut Actor<N>, connector: &C) -> Result<()> {
    let (mac, server_ip, assigned_ip) = commit::params(actor)?;
    let socket = connector.create_raw(actor.snapshot.name.as_str()).await?;
    let mut msg = [0_u8; dhcp::MAX_MESSAGE_LEN];
    let mut frame = [0_u8; dhcp::MAX_FRAME_LEN];
    match dhcp::exchange::rebind(
        &socket,
        mac,
        server_ip,
        assigned_ip,
        Scratch::new(&mut msg, &mut frame),
    )
    .await
    {
        Ok(lease) => apply_renewed_lease(actor, &lease).await,
        Err(dhcp::Failure::Nak) => {
            kmsg::warn!(
                "DHCP REBIND NAK for {}, returning to INIT",
                actor.snapshot.name
            );
            do_full_dora(actor, connector).await
        }
        Err(error) => Err(error.into()),
    }
}

/// Applies a renewed lease without changing the interface state.
async fn apply_renewed_lease<N: Ops>(actor: &mut Actor<N>, lease: &Lease) -> Result<()> {
    let index = actor.snapshot.index;
    commit::lease(actor, index, lease.clone()).await?;
    kmsg::info!("DHCP lease renewed for {}", actor.snapshot.name);

    Ok(())
}
