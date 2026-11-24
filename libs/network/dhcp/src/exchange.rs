//! Socket lifecycle and exchange bookkeeping of the `DHCPv4` method.

use alloc::format;
use core::net::Ipv4Addr;
use std::time::Instant;

use dhcp_proto::message_type;
use ip_method::{Event, SocketSlot};

use crate::Dhcp;
use crate::effects::Action;
use crate::fault::Fault;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Transmit {
    Discover,
    Select {
        server: Ipv4Addr,
        offered: Ipv4Addr,
    },
    Renew {
        server: Ipv4Addr,
        assigned: Ipv4Addr,
    },
    Rebind {
        server: Ipv4Addr,
        assigned: Ipv4Addr,
    },
}

impl Transmit {
    pub(super) const fn slot(self) -> SocketSlot {
        match self {
            Self::Renew { .. } => SocketSlot::Secondary,
            Self::Discover | Self::Select { .. } | Self::Rebind { .. } => SocketSlot::Primary,
        }
    }

    pub(super) const fn expected(self) -> u8 {
        match self {
            Self::Discover => message_type::OFFER,
            Self::Select { .. } | Self::Renew { .. } | Self::Rebind { .. } => message_type::ACK,
        }
    }

    pub(super) const fn endpoints(self) -> (Ipv4Addr, Ipv4Addr) {
        match self {
            Self::Rebind { assigned, .. } => (assigned, Ipv4Addr::BROADCAST),
            Self::Discover | Self::Select { .. } | Self::Renew { .. } => {
                (Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Exchange {
    Idle,
    Opening(Transmit),
    Entropy(Transmit),
    Reply { kind: Transmit, deadline: Instant },
}

impl Exchange {
    pub(super) const fn deadline(self) -> Option<Instant> {
        match self {
            Self::Reply { deadline, .. } => Some(deadline),
            Self::Idle | Self::Opening(_) | Self::Entropy(_) => None,
        }
    }

    pub(super) const fn kind(self) -> Option<Transmit> {
        match self {
            Self::Idle => None,
            Self::Opening(kind) | Self::Entropy(kind) | Self::Reply { kind, .. } => Some(kind),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SocketState {
    Closed,
    Opening,
    Open,
    Closing,
}

pub(super) fn begin(method: &mut Dhcp, kind: Transmit) {
    let slot = kind.slot();
    if socket(method, slot) == SocketState::Open {
        method.exchange = Exchange::Entropy(kind);
        method.queue.push_back(Action::Entropy);
        return;
    }
    store(method, slot, SocketState::Opening);
    method.exchange = Exchange::Opening(kind);
    method.queue.push_back(Action::Open(slot));
}

pub(super) fn opened(method: &mut Dhcp, slot: SocketSlot) {
    store(method, slot, SocketState::Open);
    if let Exchange::Opening(kind) = method.exchange
        && kind.slot() == slot
    {
        method.exchange = Exchange::Entropy(kind);
        method.queue.push_back(Action::Entropy);
    }
}

pub(super) const fn closed(method: &mut Dhcp, slot: SocketSlot) {
    store(method, slot, SocketState::Closed);
}

pub(super) fn release(method: &mut Dhcp, slot: SocketSlot) {
    if socket(method, slot) == SocketState::Closed {
        return;
    }
    store(method, slot, SocketState::Closing);
    method.queue.push_back(Action::Close(slot));
}

pub(super) fn release_all(method: &mut Dhcp) {
    release(method, SocketSlot::Primary);
    release(method, SocketSlot::Secondary);
}

pub(super) fn on_entropy(method: &mut Dhcp, xid: u32) {
    let Exchange::Entropy(kind) = method.exchange else {
        return;
    };
    method.xid = xid;
    method.exchange = Exchange::Idle;
    method.queue.push_back(Action::Send(kind));
}

pub(super) fn on_socket_error(method: &mut Dhcp, now: Instant, slot: SocketSlot) {
    store(method, slot, SocketState::Closed);
    let active = method.exchange.kind().filter(|kind| kind.slot() == slot);
    let Some(kind) = active else {
        let warning = format!("DHCP socket error on {slot:?} outside an exchange");
        method
            .queue
            .push_back(Action::Event(Event::Warning(warning)));
        return;
    };
    method.exchange = Exchange::Idle;
    crate::state::failure(method, now, kind, Fault::Socket);
}

pub(super) const fn socket(method: &Dhcp, slot: SocketSlot) -> SocketState {
    match slot {
        SocketSlot::Primary => method.primary,
        SocketSlot::Secondary => method.secondary,
    }
}

pub(super) const fn store(method: &mut Dhcp, slot: SocketSlot, state: SocketState) {
    match slot {
        SocketSlot::Primary => method.primary = state,
        SocketSlot::Secondary => method.secondary = state,
    }
}
