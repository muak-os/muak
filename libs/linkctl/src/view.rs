//! Point-in-time interface and address-method state.

#[cfg(feature = "ipv6")]
use alloc::vec::Vec;
use core::fmt::{Display, Formatter, Result as FmtResult};
#[cfg(feature = "ipv4")]
use core::net::Ipv4Addr;
#[cfg(feature = "ipv6")]
use core::net::Ipv6Addr;

// Maximum entries a list renders before summarising the rest.
const MAX_LISTED: usize = 4;

/// IPv4 configuration source tracked by the v4 method.
#[cfg(feature = "ipv4")]
#[derive(Debug, Clone)]
pub enum V4State {
    /// Static address configuration.
    Static(addrconf::fixed::StaticView<Ipv4Addr>),
    /// `DHCPv4` lease and phase.
    Dhcp(dhcp_client::method::View),
}

/// IPv6 configuration source tracked by the v6 method.
#[cfg(feature = "ipv6")]
#[derive(Debug, Clone)]
pub enum V6State {
    /// Static address configuration.
    Static(addrconf::fixed::StaticView<Ipv6Addr>),
    /// SLAAC-derived state.
    Slaac {
        /// Address and prefix length pairs.
        addresses: Vec<(Ipv6Addr, u8)>,
        /// Known router addresses.
        routers: Vec<Ipv6Addr>,
        /// Learned DNS servers.
        dns: Vec<Ipv6Addr>,
    },
}

#[cfg(feature = "ipv4")]
impl Display for V4State {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match *self {
            Self::Static(ref config) => write_static(
                f,
                config.address,
                config.prefix_len,
                config.gateway,
                &config.dns,
            ),
            Self::Dhcp(ref view) => {
                let lease = &view.lease;
                write!(
                    f,
                    "dhcp {}/{} server {} ",
                    lease.assigned_ip, lease.prefix_len, lease.server_ip
                )?;
                write_gateway_and_dns(f, lease.gateway, &lease.dns_servers)
            }
        }
    }
}

#[cfg(feature = "ipv6")]
impl Display for V6State {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match *self {
            Self::Static(ref config) => write_static(
                f,
                config.address,
                config.prefix_len,
                config.gateway,
                &config.dns,
            ),
            Self::Slaac {
                ref addresses,
                ref routers,
                ref dns,
            } => {
                f.write_str("slaac ")?;
                write_addresses(f, addresses)?;
                f.write_str(" routers ")?;
                write_list(f, routers)?;
                f.write_str(" dns ")?;
                write_list(f, dns)
            }
        }
    }
}

fn write_static<A: Display + Copy>(
    formatter: &mut Formatter<'_>,
    address: A,
    prefix_len: u8,
    gateway: Option<A>,
    dns: &[A],
) -> FmtResult {
    write!(formatter, "static {address}/{prefix_len} ")?;

    write_gateway_and_dns(formatter, gateway, dns)
}

fn write_gateway_and_dns<A: Display + Copy>(
    formatter: &mut Formatter<'_>,
    gateway: Option<A>,
    dns: &[A],
) -> FmtResult {
    formatter.write_str("gateway ")?;
    write_opt(formatter, gateway)?;
    formatter.write_str(" dns ")?;

    write_list(formatter, dns)
}

fn write_opt<A: Display>(formatter: &mut Formatter<'_>, value: Option<A>) -> FmtResult {
    match value {
        Some(value) => write!(formatter, "{value}"),
        None => formatter.write_str("none"),
    }
}

fn write_list<A: Display>(formatter: &mut Formatter<'_>, values: &[A]) -> FmtResult {
    let Some((first, rest)) = values.split_first() else {
        return formatter.write_str("none");
    };
    write!(formatter, "{first}")?;
    for value in rest.iter().take(MAX_LISTED - 1) {
        write!(formatter, ", {value}")?;
    }

    write_omitted(formatter, values.len())
}

#[cfg(feature = "ipv6")]
fn write_addresses(formatter: &mut Formatter<'_>, addresses: &[(Ipv6Addr, u8)]) -> FmtResult {
    let Some((&(address, prefix_len), rest)) = addresses.split_first() else {
        return formatter.write_str("none");
    };
    write!(formatter, "{address}/{prefix_len}")?;
    for &(address, prefix_len) in rest.iter().take(MAX_LISTED - 1) {
        write!(formatter, ", {address}/{prefix_len}")?;
    }

    write_omitted(formatter, addresses.len())
}

fn write_omitted(formatter: &mut Formatter<'_>, len: usize) -> FmtResult {
    let omitted = len.saturating_sub(MAX_LISTED);
    if omitted > 0 {
        write!(formatter, ", +{omitted} more")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "ipv4")]
    mod v4 {
        use alloc::{format, vec};
        use core::net::Ipv4Addr;
        use core::time::Duration;
        use std::time::SystemTime;

        use addrconf::fixed::StaticView;
        use dhcp_client::lease::Lease;
        use dhcp_client::method::{Phase, View};

        use crate::view::V4State;

        fn lease() -> Lease {
            Lease {
                obtained_at: SystemTime::UNIX_EPOCH,
                lease_time: Duration::from_hours(1),
                renewal_time: Duration::from_mins(30),
                rebind_time: Duration::from_secs(3150),
                server_ip: Ipv4Addr::new(10, 0, 0, 1),
                assigned_ip: Ipv4Addr::new(10, 0, 0, 5),
                prefix_len: 24,
                gateway: None,
                dns_servers: vec![Ipv4Addr::new(10, 0, 0, 53)],
            }
        }

        #[test]
        fn static_renders_address_gateway_and_dns() {
            // ARRANGE
            let state = V4State::Static(StaticView {
                address: Ipv4Addr::new(192, 168, 1, 10),
                prefix_len: 24,
                gateway: Some(Ipv4Addr::new(192, 168, 1, 1)),
                dns: vec![Ipv4Addr::new(1, 1, 1, 1), Ipv4Addr::new(9, 9, 9, 9)],
            });

            // ACT
            let rendered = format!("{state}");

            // ASSERT
            assert_eq!(
                rendered,
                "static 192.168.1.10/24 gateway 192.168.1.1 dns 1.1.1.1, 9.9.9.9"
            );
        }

        #[test]
        fn static_renders_absent_gateway_and_empty_dns() {
            // ARRANGE
            let state = V4State::Static(StaticView {
                address: Ipv4Addr::new(192, 168, 1, 10),
                prefix_len: 24,
                gateway: None,
                dns: Vec::new(),
            });

            // ACT
            let rendered = format!("{state}");

            // ASSERT
            assert_eq!(rendered, "static 192.168.1.10/24 gateway none dns none");
        }

        #[test]
        fn static_dns_list_is_capped() {
            // ARRANGE
            let state = V4State::Static(StaticView {
                address: Ipv4Addr::new(192, 168, 1, 10),
                prefix_len: 24,
                gateway: None,
                dns: (1..=6).map(|last| Ipv4Addr::new(1, 1, 1, last)).collect(),
            });

            // ACT
            let rendered = format!("{state}");

            // ASSERT
            assert_eq!(
                rendered,
                "static 192.168.1.10/24 gateway none dns 1.1.1.1, 1.1.1.2, 1.1.1.3, 1.1.1.4, +2 more"
            );
        }

        #[test]
        fn dhcp_renders_server_address_and_absent_gateway() {
            // ARRANGE
            let state = V4State::Dhcp(View {
                lease: lease(),
                phase: Phase::Bound,
            });

            // ACT
            let rendered = format!("{state}");

            // ASSERT
            assert_eq!(
                rendered,
                "dhcp 10.0.0.5/24 server 10.0.0.1 gateway none dns 10.0.0.53"
            );
        }
    }

    #[cfg(feature = "ipv6")]
    mod v6 {
        use alloc::{format, vec};
        use core::net::Ipv6Addr;

        use addrconf::fixed::StaticView;

        use crate::view::V6State;

        #[test]
        fn static_renders_address_gateway_and_dns() {
            // ARRANGE
            let state = V6State::Static(StaticView {
                address: Ipv6Addr::LOCALHOST,
                prefix_len: 64,
                gateway: Some(Ipv6Addr::UNSPECIFIED),
                dns: vec![Ipv6Addr::LOCALHOST],
            });

            // ACT
            let rendered = format!("{state}");

            // ASSERT
            assert_eq!(rendered, "static ::1/64 gateway :: dns ::1");
        }

        #[test]
        fn slaac_renders_addresses_routers_and_dns() {
            // ARRANGE
            let state = V6State::Slaac {
                addresses: vec![(Ipv6Addr::LOCALHOST, 64), (Ipv6Addr::UNSPECIFIED, 64)],
                routers: vec![Ipv6Addr::LOCALHOST],
                dns: vec![Ipv6Addr::UNSPECIFIED],
            };

            // ACT
            let rendered = format!("{state}");

            // ASSERT
            assert_eq!(rendered, "slaac ::1/64, ::/64 routers ::1 dns ::");
        }

        #[test]
        fn slaac_renders_empty_lists_as_none() {
            // ARRANGE
            let state = V6State::Slaac {
                addresses: Vec::new(),
                routers: Vec::new(),
                dns: Vec::new(),
            };

            // ACT
            let rendered = format!("{state}");

            // ASSERT
            assert_eq!(rendered, "slaac none routers none dns none");
        }

        #[test]
        fn slaac_lists_are_capped() {
            // ARRANGE
            let state = V6State::Slaac {
                addresses: (1..=5)
                    .map(|last| (Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, last), 64))
                    .collect(),
                routers: (1..=6)
                    .map(|last| Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, last))
                    .collect(),
                dns: (1..=4)
                    .map(|last| Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, last))
                    .collect(),
            };

            // ACT
            let rendered = format!("{state}");

            // ASSERT
            assert_eq!(
                rendered,
                "slaac ::1/64, ::2/64, ::3/64, ::4/64, +1 more routers ::1, ::2, ::3, ::4, +2 more dns ::1, ::2, ::3, ::4"
            );
        }
    }
}
