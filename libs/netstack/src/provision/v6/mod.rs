//! IPv6 address provisioning methods.

pub mod slaac;

use alloc::vec::Vec;
use core::net::{IpAddr, Ipv6Addr};

use crate::provision::fixed;
use crate::view::V6State;

/// Static IPv6 address configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// Address and prefix length pairs.
    pub addresses: Vec<(Ipv6Addr, u8)>,
    /// Optional default gateway.
    pub gateway: Option<Ipv6Addr>,
    /// DNS server addresses.
    pub dns: Vec<Ipv6Addr>,
    /// Whether stateless autoconfiguration is enabled.
    pub autoconf: bool,
}

/// IPv6 address family adapter for the static method.
pub struct V6;

impl fixed::Family for V6 {
    type Address = Ipv6Addr;
    type View = V6State;

    fn ip(address: Ipv6Addr) -> IpAddr {
        IpAddr::V6(address)
    }

    fn view(config: &fixed::Config<Ipv6Addr>, primary: (Ipv6Addr, u8)) -> V6State {
        V6State::Static(netlink::address::Ipv6Config {
            address: primary.0,
            prefix_len: primary.1,
            gateway: config.gateway,
            dns: config.dns.clone(),
        })
    }
}

/// Static IPv6 address method.
pub type Static = fixed::Static<V6>;

impl Config {
    pub(crate) fn fixed(self) -> fixed::Config<Ipv6Addr> {
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
    use core::net::Ipv6Addr;

    use super::{Config, V6};
    use crate::provision::fixed;
    use crate::view::V6State;

    const ADDRESS_A: Ipv6Addr = Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 2);
    const ADDRESS_B: Ipv6Addr = Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 3);
    const GATEWAY: Ipv6Addr = Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 1);
    const DNS: Ipv6Addr = Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 53);

    fn config() -> Config {
        Config {
            addresses: vec![(ADDRESS_A, 64), (ADDRESS_B, 64)],
            gateway: Some(GATEWAY),
            dns: vec![DNS],
            autoconf: true,
        }
    }

    #[test]
    fn fixed_drops_autoconf_and_keeps_shared_fields() {
        // ARRANGE
        let config = config();

        // ACT
        let converted = config.fixed();

        // ASSERT
        assert_eq!(converted.addresses, vec![(ADDRESS_A, 64), (ADDRESS_B, 64)]);
        assert_eq!(converted.gateway, Some(GATEWAY));
        assert_eq!(converted.dns, vec![DNS]);
    }

    #[test]
    fn family_view_maps_shared_config() {
        // ARRANGE
        let config = config().fixed();

        // ACT
        let view = <V6 as fixed::Family>::view(&config, (ADDRESS_A, 64));

        // ASSERT
        let V6State::Static(state) = view else {
            panic!("expected static view");
        };
        assert_eq!(state.address, ADDRESS_A);
        assert_eq!(state.prefix_len, 64);
        assert_eq!(state.gateway, Some(GATEWAY));
        assert_eq!(state.dns, vec![DNS]);
    }
}
