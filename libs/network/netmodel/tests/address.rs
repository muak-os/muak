//! Address and route vocabulary tests.

#[cfg(test)]
mod tests {
    use core::net::{IpAddr, Ipv4Addr};

    use netmodel::{Address, Ipv4Config, Ipv6Config, Route};

    #[test]
    fn address_holds_ip_and_prefix() {
        // ARRANGE
        let address = Address {
            address: IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)),
            prefix_len: 24,
        };

        // ACT / ASSERT
        assert_eq!(address.prefix_len, 24);
    }

    #[test]
    fn route_holds_optional_gateway() {
        // ARRANGE
        let route = Route {
            destination: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            prefix_len: 0,
            gateway: None,
        };

        // ACT / ASSERT
        assert!(route.gateway.is_none());
    }

    #[test]
    fn ipv4_config_default_is_empty() {
        // ACT
        let config = Ipv4Config::default();

        // ASSERT
        assert!(config.gateway.is_none());
        assert!(!config.dhcp);
    }

    #[test]
    fn ipv6_config_default_is_empty() {
        // ACT
        let config = Ipv6Config::default();

        // ASSERT
        assert!(config.gateway.is_none());
        assert!(!config.autoconf);
    }
}
