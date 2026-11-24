//! Reception of `DHCPv4` responses and lease binding.

use core::net::{IpAddr, Ipv4Addr};
use std::time::{Instant, SystemTime};

use dhcp_proto::{DHCP_CLIENT_PORT, DHCP_SERVER_PORT, ParsedOptions, header, validate};
use ip::v4::udp;
use ip_method::SocketSlot;

use crate::effects::Action;
use crate::exchange::{Exchange, Transmit};
use crate::fault::Fault;
use crate::state::{bind, failure};
use crate::{Dhcp, Lease};

// Handles a frame received on `slot` for the exchange waiting on it.
pub(super) fn frame(
    method: &mut Dhcp,
    now: Instant,
    wall: SystemTime,
    slot: SocketSlot,
    bytes: &[u8],
    source: Option<IpAddr>,
) {
    let Exchange::Reply { kind, .. } = method.exchange else {
        return;
    };
    if kind.slot() != slot {
        return;
    }

    match kind {
        Transmit::Discover => offered(method, now, kind, raw(bytes)),
        Transmit::Select { server, offered } => {
            selected(method, now, wall, kind, raw(bytes), server, offered);
        }
        Transmit::Rebind { server, assigned } => {
            rebound(method, now, wall, kind, raw(bytes), server, assigned);
        }
        Transmit::Renew { server, assigned } => {
            renewed(method, now, wall, kind, bytes, source, (server, assigned));
        }
    }
}

// Handles an OFFER by selecting its server and address.
fn offered(method: &mut Dhcp, now: Instant, kind: Transmit, payload: Option<&[u8]>) {
    let Some(payload) = payload else {
        return;
    };
    let Some(options) = accept(method, now, kind, payload) else {
        return;
    };
    let Some(server) = options.server_id else {
        failure(method, now, kind, Fault::MissingServerId);
        return;
    };
    let Some(offered) = assigned_address(payload, Ipv4Addr::UNSPECIFIED) else {
        failure(method, now, kind, Fault::MissingAddress);
        return;
    };
    method.exchange = Exchange::Entropy(Transmit::Select { server, offered });
    method.queue.push_back(Action::Entropy);
}

// Handles the ACK of a SELECTING request.
fn selected(
    method: &mut Dhcp,
    now: Instant,
    wall: SystemTime,
    kind: Transmit,
    payload: Option<&[u8]>,
    server: Ipv4Addr,
    offered: Ipv4Addr,
) {
    let Some(payload) = payload else {
        return;
    };
    acknowledged(method, now, wall, kind, payload, offered, server);
}

// Handles the unicast ACK of a renewal.
fn renewed(
    method: &mut Dhcp,
    now: Instant,
    wall: SystemTime,
    kind: Transmit,
    payload: &[u8],
    source: Option<IpAddr>,
    endpoints: (Ipv4Addr, Ipv4Addr),
) {
    let (server, assigned) = endpoints;
    if !source_matches(source, server) {
        return;
    }
    acknowledged(method, now, wall, kind, payload, assigned, server);
}

// Handles the broadcast ACK of a rebinding.
fn rebound(
    method: &mut Dhcp,
    now: Instant,
    wall: SystemTime,
    kind: Transmit,
    payload: Option<&[u8]>,
    server: Ipv4Addr,
    assigned: Ipv4Addr,
) {
    let Some(payload) = payload else {
        return;
    };
    acknowledged(method, now, wall, kind, payload, assigned, server);
}

// Handles an ACK by binding the lease it grants.
fn acknowledged(
    method: &mut Dhcp,
    now: Instant,
    wall: SystemTime,
    kind: Transmit,
    payload: &[u8],
    assigned: Ipv4Addr,
    server: Ipv4Addr,
) {
    let Some(options) = accept(method, now, kind, payload) else {
        return;
    };
    let Some(assigned) = assigned_address(payload, assigned) else {
        failure(method, now, kind, Fault::MissingAddress);
        return;
    };
    let lease = Lease::from_ack(wall, assigned, server, options);
    bind(method, now, kind, lease);
}

// Validates a response, reporting a NAK and ignoring unrelated messages.
fn accept(
    method: &mut Dhcp,
    now: Instant,
    kind: Transmit,
    payload: &[u8],
) -> Option<ParsedOptions> {
    match validate::response(payload, method.xid, kind.expected()) {
        Ok(options) => Some(options),
        Err(dhcp_proto::Failure::Nak) => {
            failure(method, now, kind, Fault::Nak);
            None
        }
        Err(_other) => None,
    }
}

// Strips the IPv4 and UDP headers of a frame received on the raw socket.
fn raw(bytes: &[u8]) -> Option<&[u8]> {
    let (src, dst, payload) = udp::unwrap(bytes).ok()?;
    if dst.port != DHCP_CLIENT_PORT || src.port != DHCP_SERVER_PORT {
        return None;
    }

    Some(payload)
}

// Accepts the unicast source when the transport reported one.
fn source_matches(source: Option<IpAddr>, server: Ipv4Addr) -> bool {
    source.is_none_or(|ip| ip == IpAddr::V4(server))
}

// Returns the address the response assigns, falling back when it carries no `yiaddr`.
fn assigned_address(payload: &[u8], fallback: Ipv4Addr) -> Option<Ipv4Addr> {
    let received = header::yiaddr(payload);
    if received.is_unspecified() {
        return (!fallback.is_unspecified()).then_some(fallback);
    }

    Some(received)
}
