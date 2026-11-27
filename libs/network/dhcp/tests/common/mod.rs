//! Scripted socket and frame fixtures for the DHCP method tests.

#![cfg(test)]

use core::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Instant, SystemTime};

use dhcp::Dhcp;
use dhcp_proto::{DHCP_CLIENT_PORT, DHCP_SERVER_PORT, header, message_type};
use ip::v4::{Addr, udp};
use ip_method::error::IpMethodError;
use ip_method::{Command, Effect, Event, Input, Intent, Method as _, SocketKind, SocketSlot, Step};
use netmodel::{Ipv4Config, LinkState, Mac, MethodState, Name};

/// Hardware address of the interface the method runs on.
pub(crate) const MAC: [u8; 6] = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];
/// Server that answers the exchanges.
pub(crate) const SERVER: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);
/// Gateway the server advertises.
pub(crate) const GATEWAY: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);
/// First address the server assigns.
pub(crate) const ADDRESS_A: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 2);
/// Second address the server assigns.
pub(crate) const ADDRESS_B: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 3);
/// Transaction ID the runtime supplies.
pub(crate) const XID: u32 = 0x0BAD_F00D;
/// Wall clock the method runs at.
pub(crate) const WALL: SystemTime = SystemTime::UNIX_EPOCH;

const LEASE_SECS: u32 = 100;
const YIADDR: usize = 16;
const OPTION_MESSAGE_TYPE: u8 = 53;
const OPTION_SERVER_ID: u8 = 54;
const OPTION_SUBNET_MASK: u8 = 1;
const OPTION_ROUTER: u8 = 3;
const OPTION_DNS: u8 = 6;
const OPTION_LEASE_TIME: u8 = 51;
const OPTION_END: u8 = 255;
const NAK: u8 = 6;

/// One effect the method produced while the runtime answered it.
#[derive(Debug)]
pub(crate) enum Trace {
    /// The runtime opened a socket.
    Opened(
        SocketSlot,
        SocketKind,
        Option<SocketAddr>,
        Option<SocketAddr>,
    ),
    /// The runtime closed a socket.
    Closed(SocketSlot),
    /// The runtime supplied entropy.
    Entropy,
    /// The method sent a frame.
    Sent(SocketSlot, Vec<u8>),
    /// The method emitted an event.
    Event(Event),
}

/// Link snapshot the method provisions.
pub(crate) fn link() -> ip_method::LinkInfo {
    ip_method::LinkInfo {
        name: Name::new("eth0").unwrap(),
        index: 2,
        mac: Mac::new(MAC),
        state: LinkState::Up,
    }
}

/// IPv4 configuration that enables DHCP.
pub(crate) fn config() -> Ipv4Config {
    Ipv4Config {
        dhcp: true,
        ..Ipv4Config::default()
    }
}

/// Creates a DHCP method on `eth0`.
pub(crate) fn method() -> Dhcp {
    Dhcp::new(link(), config())
}

/// Runs the method with `input`, answering every effect like the runtime does.
pub(crate) fn pump(method: &mut Dhcp, now: Instant, input: Input<'_>) -> Vec<Trace> {
    pump_with(method, now, input, XID)
}

/// Runs the method with `input`, supplying `xid` as entropy.
pub(crate) fn pump_with(method: &mut Dhcp, now: Instant, input: Input<'_>, xid: u32) -> Vec<Trace> {
    let mut trace = Vec::new();
    let mut input = input;
    for _ in 0..64_u8 {
        input = match method.step(now, WALL, input) {
            Step::Wait => break,
            Step::Effect(Effect::OpenSocket(spec)) => {
                trace.push(Trace::Opened(spec.slot, spec.kind, spec.bind, spec.peer));
                Input::SocketOpened { slot: spec.slot }
            }
            Step::Effect(Effect::CloseSocket { slot }) => {
                trace.push(Trace::Closed(slot));
                Input::SocketClosed { slot }
            }
            Step::Effect(Effect::RequestEntropy) => {
                trace.push(Trace::Entropy);
                Input::Entropy(xid)
            }
            Step::Effect(Effect::Send { slot, frame, .. }) => {
                trace.push(Trace::Sent(slot, frame.to_vec()));
                Input::Poll
            }
            Step::Effect(Effect::Emit(event)) => {
                trace.push(Trace::Event(event));
                Input::Poll
            }
        };
    }

    trace
}

/// Drives a full DORA exchange that binds `assigned`.
pub(crate) fn bind(method: &mut Dhcp, now: Instant, assigned: Ipv4Addr) {
    let trace = pump(method, now, Input::Command(Command::Start));
    let request = sent(&trace, SocketSlot::Primary).unwrap();
    let offer = offer(xid_of(&request), assigned);
    let trace = frame(method, now, SocketSlot::Primary, &offer, None);
    let request = sent(&trace, SocketSlot::Primary).unwrap();
    let ack = ack(xid_of(&request), assigned);
    drop(frame(method, now, SocketSlot::Primary, &ack, None));
}

/// Delivers a frame on `slot` as the runtime received it.
pub(crate) fn frame(
    method: &mut Dhcp,
    now: Instant,
    slot: SocketSlot,
    bytes: &[u8],
    source: Option<IpAddr>,
) -> Vec<Trace> {
    pump(
        method,
        now,
        Input::Frame {
            slot,
            bytes,
            source,
        },
    )
}

/// Builds a DHCPOFFER frame carrying `offered`.
pub(crate) fn offer(xid: u32, offered: Ipv4Addr) -> Vec<u8> {
    raw(&message(xid, message_type::OFFER, offered, &[]))
}

/// Builds a DHCPACK frame assigning `assigned`.
pub(crate) fn ack(xid: u32, assigned: Ipv4Addr) -> Vec<u8> {
    raw(&ack_payload(xid, assigned))
}

/// Builds the DHCPACK message a UDP socket delivers.
pub(crate) fn ack_payload(xid: u32, assigned: Ipv4Addr) -> Vec<u8> {
    message(xid, message_type::ACK, assigned, &[])
}

/// Builds the DHCPNAK message a UDP socket delivers.
pub(crate) fn nak_payload(xid: u32) -> Vec<u8> {
    let mut options = vec![OPTION_MESSAGE_TYPE, 1, NAK];
    options.push(OPTION_END);

    message(xid, NAK, Ipv4Addr::UNSPECIFIED, &options)
}

/// Returns the DHCP payload of a raw IPv4 frame.
pub(crate) fn payload(frame: &[u8]) -> Vec<u8> {
    let (_src, _dst, payload) = udp::unwrap(frame).unwrap();

    payload.to_vec()
}

/// Returns the DHCP message type of a DHCP payload.
pub(crate) fn kind(payload: &[u8]) -> u8 {
    let start = header::LEN + 2;

    payload.get(start).copied().unwrap_or_default()
}

/// Returns the transaction ID of a raw IPv4 frame.
pub(crate) fn xid_of(frame: &[u8]) -> u32 {
    dhcp_proto::xid::extract(&payload(frame))
}

/// Returns the first frame the method sent on `slot`.
pub(crate) fn sent(trace: &[Trace], slot: SocketSlot) -> Option<Vec<u8>> {
    trace.iter().find_map(|effect| match effect {
        &Trace::Sent(candidate, ref frame) if candidate == slot => Some(frame.clone()),
        &Trace::Opened(..)
        | &Trace::Sent(..)
        | &Trace::Closed(_)
        | &Trace::Entropy
        | &Trace::Event(_) => None,
    })
}

/// Returns the sockets the method opened, in order.
pub(crate) fn opened(
    trace: &[Trace],
) -> Vec<(
    SocketSlot,
    SocketKind,
    Option<SocketAddr>,
    Option<SocketAddr>,
)> {
    trace
        .iter()
        .filter_map(|effect| match effect {
            &Trace::Opened(slot, kind, bind, peer) => Some((slot, kind, bind, peer)),
            &Trace::Sent(..) | &Trace::Closed(_) | &Trace::Entropy | &Trace::Event(_) => None,
        })
        .collect()
}

/// Returns the sockets the method closed, in order.
pub(crate) fn closed(trace: &[Trace]) -> Vec<SocketSlot> {
    trace
        .iter()
        .filter_map(|effect| match effect {
            &Trace::Closed(slot) => Some(slot),
            &Trace::Opened(..) | &Trace::Sent(..) | &Trace::Entropy | &Trace::Event(_) => None,
        })
        .collect()
}

/// Returns the intents the method emitted, in order.
pub(crate) fn intents(trace: &[Trace]) -> Vec<Intent> {
    trace
        .iter()
        .filter_map(|effect| match effect {
            &Trace::Event(Event::Intent(intent)) => Some(intent),
            &Trace::Opened(..)
            | &Trace::Sent(..)
            | &Trace::Closed(_)
            | &Trace::Entropy
            | &Trace::Event(_) => None,
        })
        .collect()
}

/// Returns the states the method emitted, in order.
pub(crate) fn states(trace: &[Trace]) -> Vec<MethodState> {
    trace
        .iter()
        .filter_map(|effect| match effect {
            &Trace::Event(Event::State(state)) => Some(state),
            &Trace::Opened(..)
            | &Trace::Sent(..)
            | &Trace::Closed(_)
            | &Trace::Entropy
            | &Trace::Event(_) => None,
        })
        .collect()
}

/// Returns the failure the method reported, when it reported one.
pub(crate) fn failure(trace: &[Trace]) -> Option<&IpMethodError> {
    trace.iter().find_map(|effect| match effect {
        &Trace::Event(Event::Failed(ref failure)) => Some(failure),
        &Trace::Opened(..)
        | &Trace::Sent(..)
        | &Trace::Closed(_)
        | &Trace::Entropy
        | &Trace::Event(_) => None,
    })
}

/// Returns true when the method reported a warning.
pub(crate) fn warned(trace: &[Trace]) -> bool {
    trace
        .iter()
        .any(|effect| matches!(effect, &Trace::Event(Event::Warning(_))))
}

fn message(xid: u32, kind: u8, assigned: Ipv4Addr, options: &[u8]) -> Vec<u8> {
    let mut buffer = [0_u8; 1500];
    let header = header::build_with(&mut buffer, xid, MAC, Ipv4Addr::UNSPECIFIED, 0).unwrap();
    let mut message = header.to_vec();
    let end = YIADDR + 4;
    if let Some(slot) = message.get_mut(YIADDR..end) {
        slot.copy_from_slice(&assigned.octets());
    }
    message.extend_from_slice(&default_options(kind));
    message.extend_from_slice(options);

    message
}

fn default_options(kind: u8) -> Vec<u8> {
    let [s1, s2, s3, s4] = SERVER.octets();
    let mut options = vec![OPTION_MESSAGE_TYPE, 1, kind];
    options.extend([OPTION_SERVER_ID, 4, s1, s2, s3, s4]);
    options.extend([OPTION_SUBNET_MASK, 4, 255, 255, 255, 0]);
    options.extend([OPTION_ROUTER, 4, s1, s2, s3, s4]);
    options.extend([OPTION_DNS, 4, 8, 8, 8, 8]);
    options.extend([OPTION_LEASE_TIME, 4]);
    options.extend(LEASE_SECS.to_be_bytes());
    options.push(OPTION_END);

    options
}

fn raw(payload: &[u8]) -> Vec<u8> {
    let mut buffer = [0_u8; 1536];
    let src = Addr {
        ip: SERVER,
        port: DHCP_SERVER_PORT,
    };
    let dst = Addr {
        ip: Ipv4Addr::BROADCAST,
        port: DHCP_CLIENT_PORT,
    };
    let frame = udp::wrap(payload, src, dst, &mut buffer).unwrap();

    frame.to_vec()
}
