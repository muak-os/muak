//! Static IPv4 and IPv6 provisioning method.

#![warn(missing_docs)]

extern crate alloc;

mod fixed;

use alloc::collections::VecDeque;
use core::net::IpAddr;

use ip_method::Event;

/// Address configuration shared by the static methods.
#[derive(Debug, Clone)]
pub struct Config<A> {
    /// Address and prefix length pairs.
    pub addresses: Vec<(A, u8)>,
    /// Optional default gateway.
    pub gateway: Option<A>,
    /// DNS server addresses.
    pub dns: Vec<A>,
}

/// Static configuration a family exposes as its view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View<A> {
    /// Primary address.
    pub address: A,
    /// Prefix length in bits.
    pub prefix_len: u8,
    /// Optional default gateway.
    pub gateway: Option<A>,
    /// DNS server addresses.
    pub dns: Vec<A>,
}

/// Static address method generic over the concrete address type.
pub struct Static<A: Copy + Into<IpAddr>> {
    config: Config<A>,
    started: bool,
    queue: VecDeque<Event>,
}
