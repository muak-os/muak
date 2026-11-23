//! IPv4 static address provisioning.

use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr};

use crate::fixed;

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

/// IPv4 address family adapter for the static method.
pub struct V4;

impl fixed::Family for V4 {
    type Address = Ipv4Addr;

    fn ip(address: Ipv4Addr) -> IpAddr {
        IpAddr::V4(address)
    }
}

/// Static IPv4 address method.
pub type Static = fixed::Static<V4>;

impl Config {
    /// Drops the DHCP flag and keeps the fields shared by the static method.
    #[must_use = "the static method owns the converted configuration"]
    pub fn fixed(self) -> fixed::Config<Ipv4Addr> {
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

    use super::{Config, Static};
    use crate::fixed::StaticView;
    use crate::{Command, Method as _};

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
    fn static_view_maps_shared_config() {
        // ARRANGE
        let mut method = Static::new(config().fixed());

        // ACT
        method.on_command(Command::Start);
        let view = method.view().expect("view");

        // ASSERT
        assert_view(&view);
    }

    fn assert_view(view: &StaticView<Ipv4Addr>) {
        assert_eq!(view.address, ADDRESS_A);
        assert_eq!(view.prefix_len, 24);
        assert_eq!(view.gateway, Some(GATEWAY));
        assert_eq!(view.dns, vec![DNS]);
    }
}
