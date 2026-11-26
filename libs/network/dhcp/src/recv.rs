//! Reception of `DHCPv4` responses and lease binding.

use core::net::{IpAddr, Ipv4Addr};
use std::time::{Instant, SystemTime};

use dhcp_proto::{DHCP_CLIENT_PORT, DHCP_SERVER_PORT, ParsedOptions, header, validate};
use ip::v4::udp;
use ip_method::SocketSlot;

use crate::effects::Action;
use crate::exchange::{Exchange, Transmit};
use crate::fault::Fault;
use crate::{Dhcp, Lease, state};

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

fn offered(method: &mut Dhcp, now: Instant, kind: Transmit, payload: Option<&[u8]>) {
    let Some(payload) = payload else {
        return;
    };
    let Some(options) = accept(method, now, kind, payload) else {
        return;
    };
    let Some(server) = options.server_id else {
        state::failure(method, now, kind, Fault::MissingServerId);
        return;
    };
    let Some(offered) = assigned_address(payload, Ipv4Addr::UNSPECIFIED) else {
        state::failure(method, now, kind, Fault::MissingAddress);
        return;
    };
    method.exchange = Exchange::Entropy(Transmit::Select { server, offered });
    method.queue.push_back(Action::Entropy);
}

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
        state::failure(method, now, kind, Fault::MissingAddress);
        return;
    };
    let lease = Lease::from_ack(wall, assigned, server, options);
    state::bind(method, now, kind, lease);
}

fn accept(
    method: &mut Dhcp,
    now: Instant,
    kind: Transmit,
    payload: &[u8],
) -> Option<ParsedOptions> {
    match validate::response(payload, method.xid, kind.expected()) {
        Ok(options) => Some(options),
        Err(dhcp_proto::Failure::Nak) => {
            state::failure(method, now, kind, Fault::Nak);
            None
        }
        Err(_other) => None,
    }
}

fn raw(bytes: &[u8]) -> Option<&[u8]> {
    let (src, dst, payload) = udp::unwrap(bytes).ok()?;
    if dst.port != DHCP_CLIENT_PORT || src.port != DHCP_SERVER_PORT {
        return None;
    }

    Some(payload)
}

fn source_matches(source: Option<IpAddr>, server: Ipv4Addr) -> bool {
    source.is_none_or(|ip| ip == IpAddr::V4(server))
}

fn assigned_address(payload: &[u8], fallback: Ipv4Addr) -> Option<Ipv4Addr> {
    let received = header::yiaddr(payload);
    if received.is_unspecified() {
        return (!fallback.is_unspecified()).then_some(fallback);
    }

    Some(received)
}
