//! Static IPv4 and IPv6 provisioning method.

#![warn(missing_docs)]

extern crate alloc;

mod fixed;

use alloc::collections::VecDeque;
use core::net::IpAddr;

use ip_method::Event;

/// Address family adapter for [`Static`].
pub trait Family {
    /// Concrete address type for the family.
    type Address: Copy + Eq;

    /// Converts a family address into a family-neutral address.
    fn ip(address: Self::Address) -> IpAddr;
}

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

/// Static address method generic over the address family.
pub struct Static<F: Family> {
    config: Config<F::Address>,
    started: bool,
    queue: VecDeque<Event>,
}
