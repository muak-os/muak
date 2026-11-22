//! IPv4 address provisioning methods.

mod dhcp;
mod exchange;
mod intent;
mod lease;
mod retry;
mod timer;

use alloc::collections::VecDeque;
use alloc::string::String;
use core::future::Future;
use core::net::Ipv4Addr;
use std::time::Instant;

use addrconf::Event;
use dhcp::Phase;
use dhcp_client::lease::Lease;
use timer::Deadlines;

/// `DHCPv4` exchange operations used by [`Dhcp`].
pub trait Exchange: Send + Sync {
    /// Runs a full `DORA` exchange and returns the granted lease.
    fn acquire(
        &self,
        interface: &str,
        mac: [u8; 6],
    ) -> impl Future<Output = dhcp_client::Result<Lease>> + Send;

    /// Rebinds the lease by broadcasting to any server that answers.
    fn rebind(
        &self,
        interface: &str,
        mac: [u8; 6],
        server: Ipv4Addr,
        assigned: Ipv4Addr,
    ) -> impl Future<Output = dhcp_client::Result<Lease>> + Send;

    /// Renews the lease directly with the granting server.
    fn renew(
        &self,
        interface: &str,
        mac: [u8; 6],
        server: Ipv4Addr,
        assigned: Ipv4Addr,
    ) -> impl Future<Output = dhcp_client::Result<Lease>> + Send;
}

/// `DHCPv4` address method driven by lease timers.
pub struct Dhcp<E: Exchange> {
    exchange: E,
    interface: String,
    mac: [u8; 6],
    phase: Phase,
    lease: Option<Lease>,
    deadlines: Deadlines,
    retry_at: Option<Instant>,
    attempts: u32,
    queue: VecDeque<Event>,
}
