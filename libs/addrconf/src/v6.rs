//! IPv6 static address provisioning.

use alloc::vec::Vec;
use core::net::{IpAddr, Ipv6Addr};

use crate::fixed;

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

    fn ip(address: Ipv6Addr) -> IpAddr {
        IpAddr::V6(address)
    }
}

/// Static IPv6 address method.
pub type Static = fixed::Static<V6>;

impl Config {
    /// Drops the autoconf flag and keeps the fields shared by the static method.
    #[must_use = "the static method owns the converted configuration"]
    pub fn fixed(self) -> fixed::Config<Ipv6Addr> {
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

    use super::{Config, Static};
    use crate::fixed::StaticView;
    use crate::{Command, Method as _};

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
    fn static_view_maps_shared_config() {
        // ARRANGE
        let mut method = Static::new(config().fixed());

        // ACT
        method.on_command(Command::Start);
        let view = method.view().expect("view");

        // ASSERT
        assert_view(&view);
    }

    fn assert_view(view: &StaticView<Ipv6Addr>) {
        assert_eq!(view.address, ADDRESS_A);
        assert_eq!(view.prefix_len, 64);
        assert_eq!(view.gateway, Some(GATEWAY));
        assert_eq!(view.dns, vec![DNS]);
    }
}
