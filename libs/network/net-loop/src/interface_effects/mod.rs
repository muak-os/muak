//! Execution of interface effects against the runtime ports.

mod address;

use alloc::collections::VecDeque;
use std::io;

use net_observe::journal::Journal;
use net_runtime::{Entropy as _, Icmpv6 as _, Link as _, Packet as _, Udp as _};

use crate::pending::Pending;
use crate::{SocketOwner, SocketTable};

/// Executes one interface effect with the owning interface name in scope.
pub(crate) async fn apply<R: net_runtime::Runtime>(
    runtime: &R,
    interface: &netmodel::Name,
    sockets: &mut SocketTable,
    pending: &mut VecDeque<Pending>,
    journal: &mut Journal,
    effect: iface::Effect<'_>,
) -> io::Result<()> {
    match effect {
        iface::Effect::SetLinkUp { link } => runtime.link().set_up(&link).await,
        iface::Effect::SetLinkDown { link } => runtime.link().set_down(&link).await,
        iface::Effect::AddAddress {
            link,
            address,
            prefix_len,
        } => address::add(runtime, journal, link, address, prefix_len).await,
        iface::Effect::RemoveAddress {
            link,
            address,
            prefix_len,
        } => address::remove(runtime, journal, link, address, prefix_len).await,
        iface::Effect::EnsureRoute { link, gateway } => {
            address::ensure_route(runtime, journal, link, gateway).await
        }
        iface::Effect::RemoveRoute { link, gateway } => {
            address::remove_route(runtime, journal, link, gateway).await
        }
        iface::Effect::OpenSocket { family, spec } => {
            open(runtime, interface, sockets, family, spec).await
        }
        iface::Effect::Send {
            family,
            slot,
            frame,
            dst,
        } => send(runtime, interface, sockets, family, slot, frame, dst).await,
        iface::Effect::CloseSocket { family, slot } => {
            close(runtime, interface, sockets, family, slot).await
        }
        iface::Effect::RequestEntropy { family } => {
            let mut bytes = [0_u8; 4];
            runtime.entropy().fill(&mut bytes);
            pending.push_back(Pending::Entropy {
                name: interface.clone(),
                family,
                value: u32::from_le_bytes(bytes),
            });

            Ok(())
        }
        iface::Effect::Publish => Ok(()),
    }
}

async fn open<R: net_runtime::Runtime>(
    runtime: &R,
    interface: &netmodel::Name,
    sockets: &mut SocketTable,
    family: iface::Family,
    spec: ip_method::SocketSpec,
) -> io::Result<()> {
    if spec.link != *interface {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "socket spec link does not match the owning interface",
        ));
    }

    let stale = sockets
        .find(interface, family, spec.slot)
        .and_then(|id| sockets.get(id).map(|owner| (id, owner.kind)));
    if let Some((id, kind)) = stale {
        release(runtime, sockets, id, kind).await?;
    }

    let id = match spec.kind {
        ip_method::SocketKind::RawL2 => runtime.packet().open(&spec).await?,
        ip_method::SocketKind::Udp => runtime.udp().open(&spec).await?,
        ip_method::SocketKind::RawIcmpv6 => runtime.icmpv6().open(&spec).await?,
    };

    sockets.insert(
        id,
        SocketOwner {
            interface: interface.clone(),
            family,
            slot: spec.slot,
            kind: spec.kind,
        },
    );

    Ok(())
}

async fn send<R: net_runtime::Runtime>(
    runtime: &R,
    interface: &netmodel::Name,
    sockets: &SocketTable,
    family: iface::Family,
    slot: ip_method::SocketSlot,
    frame: &[u8],
    dst: Option<core::net::SocketAddr>,
) -> io::Result<()> {
    let Some(id) = sockets.find(interface, family, slot) else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no socket open for the requested slot",
        ));
    };
    let Some(kind) = sockets.get(id).map(|owner| owner.kind) else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "socket table entry disappeared during dispatch",
        ));
    };

    match kind {
        ip_method::SocketKind::RawL2 => {
            reject_destination(dst)?;
            runtime.packet().send(id, frame).await.map(|_| ())
        }
        ip_method::SocketKind::Udp => {
            reject_destination(dst)?;
            runtime.udp().send(id, frame).await.map(|_| ())
        }
        ip_method::SocketKind::RawIcmpv6 => {
            let Some(core::net::SocketAddr::V6(address)) = dst else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "ICMPv6 send requires an IPv6 destination",
                ));
            };
            runtime
                .icmpv6()
                .send(id, *address.ip(), frame)
                .await
                .map(|_| ())
        }
    }
}

async fn close<R: net_runtime::Runtime>(
    runtime: &R,
    interface: &netmodel::Name,
    sockets: &mut SocketTable,
    family: iface::Family,
    slot: ip_method::SocketSlot,
) -> io::Result<()> {
    let Some(id) = sockets.find(interface, family, slot) else {
        return Ok(());
    };
    let Some(kind) = sockets.get(id).map(|owner| owner.kind) else {
        return Ok(());
    };

    release(runtime, sockets, id, kind).await
}

async fn release<R: net_runtime::Runtime>(
    runtime: &R,
    sockets: &mut SocketTable,
    id: net_runtime::SocketId,
    kind: ip_method::SocketKind,
) -> io::Result<()> {
    sockets.remove(id);

    match kind {
        ip_method::SocketKind::RawL2 => runtime.packet().close(id).await,
        ip_method::SocketKind::Udp => runtime.udp().close(id).await,
        ip_method::SocketKind::RawIcmpv6 => runtime.icmpv6().close(id).await,
    }
}

fn reject_destination(dst: Option<core::net::SocketAddr>) -> io::Result<()> {
    if dst.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "only an ICMPv6 socket takes a destination address",
        ));
    }

    Ok(())
}
