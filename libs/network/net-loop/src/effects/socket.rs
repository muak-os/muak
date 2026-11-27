//! Socket effects executed against the runtime socket ports.

use alloc::collections::VecDeque;

use net_runtime::{Icmpv6 as _, Packet as _, Udp as _};

use crate::error::{LoopError, Result};
use crate::pending::Pending;
use crate::{SocketOwner, SocketTable};

pub(super) struct Transmit<'a> {
    pub(super) family: netmodel::Family,
    pub(super) slot: ip_method::SocketSlot,
    pub(super) frame: &'a [u8],
    pub(super) dst: Option<core::net::SocketAddr>,
}

pub(super) async fn open<R: net_runtime::Runtime>(
    runtime: &R,
    interface: &netmodel::Name,
    sockets: &mut SocketTable,
    pending: &mut VecDeque<Pending>,
    family: netmodel::Family,
    spec: ip_method::SocketSpec,
) -> Result<()> {
    if spec.link != *interface {
        return Err(LoopError::SocketSpecLink);
    }
    let slot = spec.slot;
    let stale = sockets
        .find(interface, family, slot)
        .and_then(|id| sockets.get(id).map(|owner| (id, owner.kind)));
    if let Some((id, kind)) = stale {
        drop(release(runtime, sockets, id, kind).await);
    }

    let opened = match spec.kind {
        ip_method::SocketKind::RawL2 => runtime.packet().open(&spec).await,
        ip_method::SocketKind::Udp => runtime.udp().open(&spec).await,
        ip_method::SocketKind::RawIcmpv6 => runtime.icmpv6().open(&spec).await,
    };
    let id = match opened {
        Ok(id) => id,
        Err(error) => {
            let failed = Pending::SocketError {
                name: interface.clone(),
                family,
                slot,
                error: error.into(),
            };
            pending.push_back(failed);
            return Ok(());
        }
    };
    let owner = SocketOwner {
        interface: interface.clone(),
        family,
        slot,
        kind: spec.kind,
    };
    sockets.insert(id, owner);
    let opened = Pending::SocketOpened {
        name: interface.clone(),
        family,
        slot,
    };
    pending.push_back(opened);

    Ok(())
}

pub(super) async fn send<R: net_runtime::Runtime>(
    runtime: &R,
    interface: &netmodel::Name,
    sockets: &SocketTable,
    pending: &mut VecDeque<Pending>,
    transmit: Transmit<'_>,
) -> Result<()> {
    let family = transmit.family;
    let slot = transmit.slot;
    let outcome = write(runtime, sockets, interface, transmit).await;
    let queued = match outcome {
        Ok(()) => Pending::SendCompleted {
            name: interface.clone(),
            family,
            slot,
        },
        Err(error) => Pending::SocketError {
            name: interface.clone(),
            family,
            slot,
            error: error.into(),
        },
    };
    pending.push_back(queued);

    Ok(())
}

pub(super) async fn close<R: net_runtime::Runtime>(
    runtime: &R,
    interface: &netmodel::Name,
    sockets: &mut SocketTable,
    pending: &mut VecDeque<Pending>,
    family: netmodel::Family,
    slot: ip_method::SocketSlot,
) -> Result<()> {
    let outcome = match sockets.find(interface, family, slot) {
        Some(id) => match sockets.get(id).map(|owner| owner.kind) {
            Some(kind) => release(runtime, sockets, id, kind).await,
            None => Ok(()),
        },
        None => Ok(()),
    };
    let queued = match outcome {
        Ok(()) => Pending::SocketClosed {
            name: interface.clone(),
            family,
            slot,
        },
        Err(error) => Pending::SocketError {
            name: interface.clone(),
            family,
            slot,
            error: error.into(),
        },
    };
    pending.push_back(queued);

    Ok(())
}

async fn write<R: net_runtime::Runtime>(
    runtime: &R,
    sockets: &SocketTable,
    interface: &netmodel::Name,
    transmit: Transmit<'_>,
) -> Result<()> {
    let Some(id) = sockets.find(interface, transmit.family, transmit.slot) else {
        return Err(LoopError::SocketMissing);
    };
    let Some(kind) = sockets.get(id).map(|owner| owner.kind) else {
        return Err(LoopError::SocketTableMissing);
    };

    match kind {
        ip_method::SocketKind::RawL2 => {
            reject_destination(transmit.dst)?;
            runtime.packet().send(id, transmit.frame).await?;
        }
        ip_method::SocketKind::Udp => {
            reject_destination(transmit.dst)?;
            runtime.udp().send(id, transmit.frame).await?;
        }
        ip_method::SocketKind::RawIcmpv6 => {
            let Some(core::net::SocketAddr::V6(address)) = transmit.dst else {
                return Err(LoopError::MissingIcmpv6Destination);
            };
            runtime
                .icmpv6()
                .send(id, *address.ip(), transmit.frame)
                .await?;
        }
    }

    Ok(())
}

/// Removes a socket from the table and closes its runtime handle.
pub(crate) async fn release<R: net_runtime::Runtime>(
    runtime: &R,
    sockets: &mut SocketTable,
    id: net_runtime::SocketId,
    kind: ip_method::SocketKind,
) -> Result<()> {
    sockets.remove(id);

    match kind {
        ip_method::SocketKind::RawL2 => runtime.packet().close(id).await?,
        ip_method::SocketKind::Udp => runtime.udp().close(id).await?,
        ip_method::SocketKind::RawIcmpv6 => runtime.icmpv6().close(id).await?,
    }

    Ok(())
}

const fn reject_destination(dst: Option<core::net::SocketAddr>) -> Result<()> {
    if dst.is_some() {
        return Err(LoopError::UnexpectedDestination);
    }

    Ok(())
}
