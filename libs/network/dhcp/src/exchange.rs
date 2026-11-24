//! Socket lifecycle and exchange bookkeeping of the `DHCPv4` method.

use alloc::format;
use core::net::Ipv4Addr;
use std::time::Instant;

use dhcp_proto::message_type;
use ip_method::{Event, SocketSlot};

use crate::Dhcp;
use crate::effects::Action;
use crate::fault::Fault;

// Message one exchange sends before waiting for a reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Transmit {
    // A DISCOVER looking for any server.
    Discover,
    // A REQUEST selecting the server that offered the address.
    Select {
        // Server that offered the lease.
        server: Ipv4Addr,
        // Address the server offered.
        offered: Ipv4Addr,
    },
    // A unicast REQUEST renewing the lease with the granting server.
    Renew {
        // Server that granted the lease.
        server: Ipv4Addr,
        // Address the lease assigns.
        assigned: Ipv4Addr,
    },
    // A broadcast REQUEST rebinding the lease with any server.
    Rebind {
        // Server that granted the lease.
        server: Ipv4Addr,
        // Address the lease assigns.
        assigned: Ipv4Addr,
    },
}

impl Transmit {
    // Socket the message travels on.
    pub(super) const fn slot(self) -> SocketSlot {
        match self {
            Self::Renew { .. } => SocketSlot::Secondary,
            Self::Discover | Self::Select { .. } | Self::Rebind { .. } => SocketSlot::Primary,
        }
    }

    // DHCP message type the reply must carry.
    pub(super) const fn expected(self) -> u8 {
        match self {
            Self::Discover => message_type::OFFER,
            Self::Select { .. } | Self::Renew { .. } | Self::Rebind { .. } => message_type::ACK,
        }
    }

    // Source and destination the IPv4 header carries on the raw socket.
    //
    // Renew travels unframed on the UDP socket, so its arm mirrors the broadcast
    // default only to keep the match total.
    pub(super) const fn endpoints(self) -> (Ipv4Addr, Ipv4Addr) {
        match self {
            Self::Rebind { assigned, .. } => (assigned, Ipv4Addr::BROADCAST),
            Self::Discover | Self::Select { .. } | Self::Renew { .. } => {
                (Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST)
            }
        }
    }
}

// Step of one exchange the method is waiting on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Exchange {
    // No exchange is in flight.
    Idle,
    // Waiting for the socket that carries `kind` to open.
    Opening(Transmit),
    // Waiting for the transaction ID that starts `kind`.
    Entropy(Transmit),
    // Waiting for a reply to `kind` until the deadline.
    Reply {
        // Message the exchange sent.
        kind: Transmit,
        // Instant after which the reply is a timeout.
        deadline: Instant,
    },
}

impl Exchange {
    // Wake deadline of the exchange, when it waits for a reply.
    pub(super) const fn deadline(self) -> Option<Instant> {
        match self {
            Self::Reply { deadline, .. } => Some(deadline),
            Self::Idle | Self::Opening(_) | Self::Entropy(_) => None,
        }
    }

    // Message the exchange is waiting to send or answer.
    pub(super) const fn kind(self) -> Option<Transmit> {
        match self {
            Self::Idle => None,
            Self::Opening(kind) | Self::Entropy(kind) | Self::Reply { kind, .. } => Some(kind),
        }
    }
}

// State of one socket the method owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SocketState {
    // No socket is open in this slot.
    Closed,
    // The runtime is opening the socket.
    Opening,
    // The socket is open and may carry a send.
    Open,
    // The runtime is closing the socket.
    Closing,
}

// Queues the effects that start `kind`, reusing an open socket.
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

// Records that the runtime opened `slot` and queues the entropy request.
pub(super) fn opened(method: &mut Dhcp, slot: SocketSlot) {
    store(method, slot, SocketState::Open);
    if let Exchange::Opening(kind) = method.exchange
        && kind.slot() == slot
    {
        method.exchange = Exchange::Entropy(kind);
        method.queue.push_back(Action::Entropy);
    }
}

// Records that the runtime closed `slot`.
pub(super) const fn closed(method: &mut Dhcp, slot: SocketSlot) {
    store(method, slot, SocketState::Closed);
}

// Queues closing `slot` unless it is already closed.
pub(super) fn release(method: &mut Dhcp, slot: SocketSlot) {
    if socket(method, slot) == SocketState::Closed {
        return;
    }
    store(method, slot, SocketState::Closing);
    method.queue.push_back(Action::Close(slot));
}

// Queues closing every socket the method owns.
pub(super) fn release_all(method: &mut Dhcp) {
    release(method, SocketSlot::Primary);
    release(method, SocketSlot::Secondary);
}

// Stores `xid` and queues the message the exchange was waiting to send.
pub(super) fn on_entropy(method: &mut Dhcp, xid: u32) {
    let Exchange::Entropy(kind) = method.exchange else {
        return;
    };
    method.xid = xid;
    method.exchange = Exchange::Idle;
    method.queue.push_back(Action::Send(kind));
}

// Routes a socket error to the exchange that needed it, warning otherwise.
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

// Maps a socket slot to the state stored for it.
pub(super) const fn socket(method: &Dhcp, slot: SocketSlot) -> SocketState {
    match slot {
        SocketSlot::Primary => method.primary,
        SocketSlot::Secondary => method.secondary,
    }
}

// Stores the state of the socket in `slot`.
pub(super) const fn store(method: &mut Dhcp, slot: SocketSlot, state: SocketState) {
    match slot {
        SocketSlot::Primary => method.primary = state,
        SocketSlot::Secondary => method.secondary = state,
    }
}
