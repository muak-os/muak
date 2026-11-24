//! Rendering of the queued effects of the `DHCPv4` method.

use core::net::SocketAddr;
use std::time::Instant;

use dhcp_proto::{DHCP_CLIENT_PORT, DHCP_SERVER_PORT, builder};
use ip::v4::udp;
use ip_method::{Effect, Event, SocketKind, SocketSlot, SocketSpec, Step};

use crate::exchange::{Exchange, SocketState, Transmit, store};
use crate::fault::Fault;
use crate::state::degrade;
use crate::{Dhcp, RESPONSE_TIMEOUT};

// One effect the method still owes the runtime.
pub(super) enum Action {
    // Open the socket in this slot.
    Open(SocketSlot),
    // Send this message on its slot.
    Send(Transmit),
    // Close the socket in this slot.
    Close(SocketSlot),
    // Request entropy for a transaction ID.
    Entropy,
    // Emit this event to the controller.
    Event(Event),
}

// Returns the earliest effect still queued as one step.
pub(super) fn emit(method: &mut Dhcp, now: Instant) -> Step<'_> {
    match method.queue.pop_front() {
        Some(Action::Event(event)) => Step::Effect(Effect::Emit(event)),
        Some(Action::Open(slot)) => open(method, slot),
        Some(Action::Close(slot)) => Step::Effect(Effect::CloseSocket { slot }),
        Some(Action::Entropy) => Step::Effect(Effect::RequestEntropy),
        Some(Action::Send(kind)) => send(method, now, kind),
        None => Step::Wait,
    }
}

// Renders the open effect for `slot`, or gives the exchange up when it has no lease.
fn open(method: &mut Dhcp, slot: SocketSlot) -> Step<'_> {
    if let Some(spec) = spec(method, slot) {
        return Step::Effect(Effect::OpenSocket(spec));
    }
    store(method, slot, SocketState::Closed);
    method.exchange = Exchange::Idle;
    Step::Wait
}

// Describes the socket the runtime must open for `slot`.
fn spec(method: &Dhcp, slot: SocketSlot) -> Option<SocketSpec> {
    let link = method.link.name.clone();
    if slot == SocketSlot::Primary {
        return Some(SocketSpec {
            slot,
            kind: SocketKind::RawL2,
            link,
            bind: None,
            peer: None,
        });
    }
    let lease = method.lease.as_ref()?;

    Some(SocketSpec {
        slot,
        kind: SocketKind::Udp,
        link,
        bind: Some(SocketAddr::from((lease.assigned_ip, DHCP_CLIENT_PORT))),
        peer: Some(SocketAddr::from((lease.server_ip, DHCP_SERVER_PORT))),
    })
}

// Serializes `kind` into the method buffers and renders its send effect.
fn send(method: &mut Dhcp, now: Instant, kind: Transmit) -> Step<'_> {
    let mac = method.link.mac.into_array();
    let xid = method.xid;
    let encoded = match kind {
        Transmit::Discover => builder::discover(&mut method.message, xid, mac),
        Transmit::Select { server, offered } => builder::request(
            &mut method.message,
            xid,
            mac,
            offered,
            builder::Mode::Selecting { server },
        ),
        Transmit::Renew { assigned, .. } => builder::request(
            &mut method.message,
            xid,
            mac,
            assigned,
            builder::Mode::Renew,
        ),
        Transmit::Rebind { assigned, .. } => builder::request(
            &mut method.message,
            xid,
            mac,
            assigned,
            builder::Mode::Rebind,
        ),
    };
    let Ok(message) = encoded else {
        degrade(method, Fault::Encoding);
        return Step::Wait;
    };
    let len = message.len();
    let Some(payload) = method.message.get(..len) else {
        degrade(method, Fault::Encoding);
        return Step::Wait;
    };
    let deadline = now.checked_add(RESPONSE_TIMEOUT).unwrap_or(now);
    method.exchange = Exchange::Reply { kind, deadline };

    if kind.slot() == SocketSlot::Secondary {
        return Step::Effect(Effect::Send {
            slot: SocketSlot::Secondary,
            frame: payload,
            dst: None,
        });
    }
    let (src, dst) = kind.endpoints();
    let src = ip::v4::Addr {
        ip: src,
        port: DHCP_CLIENT_PORT,
    };
    let dst = ip::v4::Addr {
        ip: dst,
        port: DHCP_SERVER_PORT,
    };
    if let Ok(frame) = udp::wrap(payload, src, dst, &mut method.frame) {
        return Step::Effect(Effect::Send {
            slot: SocketSlot::Primary,
            frame,
            dst: None,
        });
    }
    degrade(method, Fault::Encoding);
    Step::Wait
}
