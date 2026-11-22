//! IPv4 address provisioning methods.

mod dhcp;
mod exchange;
mod intent;
mod lease;
mod retry;
mod timer;

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;
use core::future::Future;
use core::net::{IpAddr, Ipv4Addr};
use std::time::Instant;

use dhcp::Phase;
use dhcp_client::lease::Lease;
use timer::Deadlines;

use crate::provision::{Event, fixed};
use crate::view::V4State;

/// Static IPv4 address configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// Address and prefix length pairs.
    pub addresses: Vec<(Ipv4Addr, u8)>,
    /// Optional default gateway.
    pub gateway: Option<Ipv4Addr>,
    /// DNS server addresses.
    pub dns: Vec<Ipv4Addr>,
    /// Whether the interface acquires its address over `DHCPv4`.
    pub dhcp: bool,
}

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

/// IPv4 address family adapter for the static method.
pub struct V4;

impl fixed::Family for V4 {
    type Address = Ipv4Addr;
    type View = V4State;

    fn ip(address: Ipv4Addr) -> IpAddr {
        IpAddr::V4(address)
    }

    fn view(config: &fixed::Config<Ipv4Addr>, primary: (Ipv4Addr, u8)) -> V4State {
        V4State::Static(netlink::address::IpConfig {
            address: primary.0,
            prefix_len: primary.1,
            gateway: config.gateway,
            dns: config.dns.clone(),
        })
    }
}

/// Static IPv4 address method.
pub type Static = fixed::Static<V4>;

impl Config {
    pub(crate) fn fixed(self) -> fixed::Config<Ipv4Addr> {
        fixed::Config {
            addresses: self.addresses,
            gateway: self.gateway,
            dns: self.dns,
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use core::net::Ipv4Addr;

    use super::{Config, V4};
    use crate::provision::fixed;
    use crate::view::V4State;

    const ADDRESS_A: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 2);
    const ADDRESS_B: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 3);
    const GATEWAY: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);
    const DNS: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 53);

    fn config() -> Config {
        Config {
            addresses: vec![(ADDRESS_A, 24), (ADDRESS_B, 24)],
            gateway: Some(GATEWAY),
            dns: vec![DNS],
            dhcp: false,
        }
    }

    #[test]
    fn fixed_drops_dhcp_and_keeps_shared_fields() {
        // ARRANGE
        let config = config();

        // ACT
        let converted = config.fixed();

        // ASSERT
        assert_eq!(converted.addresses, vec![(ADDRESS_A, 24), (ADDRESS_B, 24)]);
        assert_eq!(converted.gateway, Some(GATEWAY));
        assert_eq!(converted.dns, vec![DNS]);
    }

    #[test]
    fn family_view_maps_shared_config() {
        // ARRANGE
        let config = config().fixed();

        // ACT
        let view = <V4 as fixed::Family>::view(&config, (ADDRESS_A, 24));

        // ASSERT
        let V4State::Static(state) = view else {
            panic!("expected static view");
        };
        assert_eq!(state.address, ADDRESS_A);
        assert_eq!(state.prefix_len, 24);
        assert_eq!(state.gateway, Some(GATEWAY));
        assert_eq!(state.dns, vec![DNS]);
    }
}
