//! `DHCPv4` address method driven by RFC 2131 lease timers.

mod dhcp;
mod intent;
mod operations;
mod retry;
mod state;
mod timer;

use alloc::collections::VecDeque;
use alloc::string::String;
use core::future::Future;
use core::net::Ipv4Addr;
use std::time::Instant;

use addrconf::Event;

use self::timer::Deadlines;
use crate::lease::Lease;

/// `DHCPv4` exchange operations used by [`Dhcp`].
pub trait Operations: Send + Sync {
    /// Runs a full `DORA` exchange and returns the granted lease.
    fn acquire(
        &self,
        interface: &str,
        mac: [u8; 6],
    ) -> impl Future<Output = crate::Result<Lease>> + Send;

    /// Rebinds the lease by broadcasting to any server that answers.
    fn rebind(
        &self,
        interface: &str,
        mac: [u8; 6],
        server: Ipv4Addr,
        assigned: Ipv4Addr,
    ) -> impl Future<Output = crate::Result<Lease>> + Send;

    /// Renews the lease directly with the granting server.
    fn renew(
        &self,
        interface: &str,
        mac: [u8; 6],
        server: Ipv4Addr,
        assigned: Ipv4Addr,
    ) -> impl Future<Output = crate::Result<Lease>> + Send;
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

/// `DHCPv4` address method driven by lease timers.
pub struct Dhcp<O: Operations> {
    operations: O,
    interface: String,
    mac: [u8; 6],
    phase: Phase,
    lease: Option<Lease>,
    deadlines: Deadlines,
    retry_at: Option<Instant>,
    attempts: u32,
    queue: VecDeque<Event>,
}
