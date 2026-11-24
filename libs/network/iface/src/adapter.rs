//! IP address family adapters for the shared static method.

use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// IPv4 address family adapter.
pub(crate) struct V4;

impl ip_static::Family for V4 {
    type Address = Ipv4Addr;

    fn ip(address: Ipv4Addr) -> IpAddr {
        IpAddr::V4(address)
    }
}

/// IPv6 address family adapter.
pub(crate) struct V6;

impl ip_static::Family for V6 {
    type Address = Ipv6Addr;

    fn ip(address: Ipv6Addr) -> IpAddr {
        IpAddr::V6(address)
    }
}
