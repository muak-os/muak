//! RFC 2131 `DHCPv4` client finite state machine.

#![warn(missing_docs)]

extern crate alloc;

mod acquire;
mod effects;
mod exchange;
mod fault;
mod intent;
mod lease;
mod lifecycle;
mod method;
mod recv;
mod retry;
mod state;
mod timer;

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::net::Ipv4Addr;
use core::time::Duration;
use std::time::{Instant, SystemTime};

use effects::Action;
use exchange::{Exchange, SocketState};
use timer::Deadlines;

// Capacity of the internal DHCP message buffer.
const MAX_MESSAGE_LEN: usize = 1500;
// Capacity of the internal IPv4 and UDP frame buffer.
const MAX_FRAME_LEN: usize = 1536;
// Time a server has to answer one exchange message.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);

/// A successfully acquired `DHCPv4` lease with timing and server metadata.
#[derive(Debug, Clone)]
pub struct Lease {
    /// Wall clock time the lease was obtained at.
    pub obtained_at: SystemTime,
    /// Total lease duration granted by the server.
    pub lease_time: Duration,
    /// Time after acquisition at which to renew.
    pub renewal_time: Duration,
    /// Time after acquisition at which to rebind.
    pub rebind_time: Duration,
    /// Address of the server that granted the lease.
    pub server_ip: Ipv4Addr,
    /// Address assigned to the interface.
    pub assigned_ip: Ipv4Addr,
    /// Prefix length of the assigned subnet.
    pub prefix_len: u8,
    /// Default gateway, when the server advertised one.
    pub gateway: Option<Ipv4Addr>,
    /// DNS servers advertised by the server.
    pub dns_servers: Vec<Ipv4Addr>,
}

/// Lifecycle phase of the `DHCPv4` method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The method has not started.
    Idle,
    /// A `DORA` exchange is in flight, possibly waiting for a retry deadline.
    Acquiring,
    /// The lease is held and its renewal deadline is armed.
    Bound,
    /// The lease is being renewed with the granting server.
    Renewing,
    /// The lease is being rebound by broadcast.
    Rebinding,
    /// The link went down or acquisition failed, keeping the lease when there is one.
    Degraded,
    /// The method is stopped and holds no lease.
    Stopped,
}

/// Lease and lifecycle phase the method reports to the controller.
#[derive(Debug, Clone)]
pub struct View {
    /// Lease currently held.
    pub lease: Lease,
    /// Lifecycle phase of the method.
    pub phase: Phase,
}

/// `DHCPv4` address method driven by RFC 2131 lease timers.
pub struct Dhcp {
    link: ip_method::LinkInfo,
    config: netmodel::Ipv4Config,
    phase: Phase,
    lease: Option<Lease>,
    // Instant the held lease was bound at, anchoring its deadlines.
    bound_at: Option<Instant>,
    deadlines: Deadlines,
    retry_at: Option<Instant>,
    attempts: u32,
    exchange: Exchange,
    primary: SocketState,
    secondary: SocketState,
    xid: u32,
    queue: VecDeque<Action>,
    message: [u8; MAX_MESSAGE_LEN],
    frame: [u8; MAX_FRAME_LEN],
}
