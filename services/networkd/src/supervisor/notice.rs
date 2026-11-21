//! Interface controller notices translated into kernel log lines.

use core::fmt::{Display, Write as _};

use netlink::link::ifname::Name;
use netstack::interface::Notice;
#[cfg(feature = "ipv4")]
use netstack::interface::V4Notice;
#[cfg(feature = "ipv6")]
use netstack::interface::V6Notice;
use netstack::provision;
#[cfg(feature = "ipv4")]
use netstack::view::V4State;
#[cfg(feature = "ipv6")]
use netstack::view::V6State;
use tokio::sync::mpsc;

// Placeholder rendered for an absent gateway, router or server.
const ABSENT: &str = "none";

// Maximum entries a joined list renders before summarising the rest.
const MAX_LISTED: usize = 4;

// Kernel log priority a described notice maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Info,
    Warn,
}

/// Logs IPv4 notices until the controller drops its sender.
#[cfg(feature = "ipv4")]
pub(super) async fn log_v4(name: Name, rx: mpsc::Receiver<V4Notice>) {
    log(name, rx, describe_v4).await;
}

/// Logs IPv6 notices until the controller drops its sender.
#[cfg(feature = "ipv6")]
pub(super) async fn log_v6(name: Name, rx: mpsc::Receiver<V6Notice>) {
    log(name, rx, describe_v6).await;
}

async fn log<V>(
    name: Name,
    mut rx: mpsc::Receiver<Notice<V>>,
    describe: fn(Notice<V>) -> Option<(Level, String)>,
) {
    while let Some(notice) = rx.recv().await {
        let Some((level, text)) = describe(notice) else {
            continue;
        };
        match level {
            Level::Info => kmsg::info!("{name}: {text}"),
            Level::Warn => kmsg::warn!("{name}: {text}"),
        }
    }
}

#[cfg(feature = "ipv4")]
fn describe_v4(notice: V4Notice) -> Option<(Level, String)> {
    match notice {
        V4Notice::State(provision::State::Bound) => None,
        V4Notice::State(state) => Some((Level::Info, format!("IPv4 state {}", phase(state)))),
        V4Notice::Configured(state) => Some((Level::Info, format!("IPv4 {}", format_v4(&state)))),
        V4Notice::Warning(warning) => Some((Level::Warn, format!("IPv4 {warning}"))),
    }
}

#[cfg(feature = "ipv6")]
fn describe_v6(notice: V6Notice) -> Option<(Level, String)> {
    match notice {
        V6Notice::State(provision::State::Bound) => None,
        V6Notice::State(state) => Some((Level::Info, format!("IPv6 state {}", phase(state)))),
        V6Notice::Configured(state) => Some((Level::Info, format!("IPv6 {}", format_v6(&state)))),
        V6Notice::Warning(warning) => Some((Level::Warn, format!("IPv6 {warning}"))),
    }
}

#[cfg(feature = "ipv4")]
fn format_v4(state: &V4State) -> String {
    match *state {
        V4State::Static(ref config) => format!(
            "static {}/{} gateway {} dns {}",
            config.address,
            config.prefix_len,
            or_absent(config.gateway),
            join(&config.dns),
        ),
        V4State::Dhcp { ref lease, .. } => format!(
            "dhcp {}/{} server {} gateway {} dns {}",
            lease.assigned_ip,
            lease.prefix_len,
            lease.server_ip,
            or_absent(lease.gateway),
            join(&lease.dns_servers),
        ),
    }
}

#[cfg(feature = "ipv6")]
fn format_v6(state: &V6State) -> String {
    match *state {
        V6State::Static(ref config) => format!(
            "static {}/{} gateway {} dns {}",
            config.address,
            config.prefix_len,
            or_absent(config.gateway),
            join(&config.dns),
        ),
        V6State::Slaac {
            ref addresses,
            ref routers,
            ref dns,
        } => format!(
            "slaac {} routers {} dns {}",
            join_with(addresses, |&(address, prefix_len)| format!(
                "{address}/{prefix_len}"
            )),
            join(routers),
            join(dns),
        ),
    }
}

fn or_absent<A: Display>(value: Option<A>) -> String {
    value.map_or_else(|| ABSENT.to_owned(), |value| value.to_string())
}

fn join<A: Display>(values: &[A]) -> String {
    join_with(values, ToString::to_string)
}

fn join_with<A>(values: &[A], render: fn(&A) -> String) -> String {
    let Some((first, rest)) = values.split_first() else {
        return ABSENT.to_owned();
    };
    let mut rendered = render(first);
    for value in rest.iter().take(MAX_LISTED - 1) {
        rendered.push_str(", ");
        rendered.push_str(&render(value));
    }
    let omitted = values.len().saturating_sub(MAX_LISTED);
    if omitted > 0 {
        let _result = write!(rendered, ", +{omitted} more");
    }
    rendered
}

const fn phase(state: provision::State) -> &'static str {
    match state {
        provision::State::Acquiring => "acquiring",
        provision::State::Bound => "bound",
        provision::State::Degraded => "degraded",
        provision::State::Stopped => "stopped",
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "ipv4")]
    mod v4 {
        use core::net::Ipv4Addr;
        use core::time::Duration;
        use std::time::SystemTime;

        use dhcp_client::lease::Lease;
        use netlink::address::IpConfig;
        use netstack::interface::V4Notice;
        use netstack::provision;
        use netstack::view::V4State;

        use super::super::{Level, describe_v4};

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
                dns_servers: Vec::new(),
            }
        }

        #[test]
        fn static_configuration_renders_address_gateway_and_dns() {
            // ARRANGE
            let notice = V4Notice::Configured(V4State::Static(IpConfig {
                address: Ipv4Addr::new(192, 168, 1, 10),
                prefix_len: 24,
                gateway: Some(Ipv4Addr::new(192, 168, 1, 1)),
                dns: vec![Ipv4Addr::new(1, 1, 1, 1), Ipv4Addr::new(9, 9, 9, 9)],
            }));

            // ACT
            let described = describe_v4(notice);

            // ASSERT
            assert_eq!(
                described,
                Some((
                    Level::Info,
                    "IPv4 static 192.168.1.10/24 gateway 192.168.1.1 dns 1.1.1.1, 9.9.9.9"
                        .to_owned()
                ))
            );
        }

        #[test]
        fn long_dns_list_is_capped() {
            // ARRANGE
            let notice = V4Notice::Configured(V4State::Static(IpConfig {
                address: Ipv4Addr::new(192, 168, 1, 10),
                prefix_len: 24,
                gateway: None,
                dns: vec![
                    Ipv4Addr::new(1, 1, 1, 1),
                    Ipv4Addr::new(2, 2, 2, 2),
                    Ipv4Addr::new(3, 3, 3, 3),
                    Ipv4Addr::new(4, 4, 4, 4),
                    Ipv4Addr::new(5, 5, 5, 5),
                    Ipv4Addr::new(6, 6, 6, 6),
                ],
            }));

            // ACT
            let described = describe_v4(notice);

            // ASSERT
            assert_eq!(
                described,
                Some((
                    Level::Info,
                    "IPv4 static 192.168.1.10/24 gateway none dns 1.1.1.1, 2.2.2.2, 3.3.3.3, 4.4.4.4, +2 more"
                        .to_owned()
                ))
            );
        }

        #[test]
        fn dhcp_configuration_renders_server_and_absent_gateway() {
            // ARRANGE
            let notice = V4Notice::Configured(V4State::Dhcp {
                lease: lease(),
                phase: netstack::view::DhcpState::Bound,
            });

            // ACT
            let described = describe_v4(notice);

            // ASSERT
            assert_eq!(
                described,
                Some((
                    Level::Info,
                    "IPv4 dhcp 10.0.0.5/24 server 10.0.0.1 gateway none dns none".to_owned()
                ))
            );
        }

        #[test]
        fn bound_state_is_not_described() {
            // ARRANGE
            let notice = V4Notice::State(provision::State::Bound);

            // ACT
            let described = describe_v4(notice);

            // ASSERT
            assert_eq!(described, None);
        }

        #[test]
        fn degraded_state_is_described_at_info() {
            // ARRANGE
            let notice = V4Notice::State(provision::State::Degraded);

            // ACT
            let described = describe_v4(notice);

            // ASSERT
            assert_eq!(
                described,
                Some((Level::Info, "IPv4 state degraded".to_owned()))
            );
        }

        #[test]
        fn warning_is_described_at_warn() {
            // ARRANGE
            let notice = V4Notice::Warning("DHCP renew failed on eth0".to_owned());

            // ACT
            let described = describe_v4(notice);

            // ASSERT
            assert_eq!(
                described,
                Some((Level::Warn, "IPv4 DHCP renew failed on eth0".to_owned()))
            );
        }
    }

    #[cfg(feature = "ipv6")]
    mod v6 {
        use core::net::Ipv6Addr;

        use netlink::address::Ipv6Config;
        use netstack::interface::V6Notice;
        use netstack::provision;
        use netstack::view::V6State;

        use super::super::{Level, describe_v6};

        #[test]
        fn static_configuration_renders_address_gateway_and_dns() {
            // ARRANGE
            let notice = V6Notice::Configured(V6State::Static(Ipv6Config {
                address: Ipv6Addr::LOCALHOST,
                prefix_len: 64,
                gateway: Some(Ipv6Addr::UNSPECIFIED),
                dns: vec![Ipv6Addr::LOCALHOST],
            }));

            // ACT
            let described = describe_v6(notice);

            // ASSERT
            assert_eq!(
                described,
                Some((
                    Level::Info,
                    "IPv6 static ::1/64 gateway :: dns ::1".to_owned()
                ))
            );
        }

        #[test]
        fn slaac_configuration_renders_every_address_routers_and_dns() {
            // ARRANGE
            let notice = V6Notice::Configured(V6State::Slaac {
                addresses: vec![(Ipv6Addr::LOCALHOST, 64), (Ipv6Addr::UNSPECIFIED, 64)],
                routers: vec![Ipv6Addr::LOCALHOST],
                dns: vec![Ipv6Addr::UNSPECIFIED],
            });

            // ACT
            let described = describe_v6(notice);

            // ASSERT
            assert_eq!(
                described,
                Some((
                    Level::Info,
                    "IPv6 slaac ::1/64, ::/64 routers ::1 dns ::".to_owned()
                ))
            );
        }

        #[test]
        fn empty_slaac_configuration_renders_none() {
            // ARRANGE
            let notice = V6Notice::Configured(V6State::Slaac {
                addresses: Vec::new(),
                routers: Vec::new(),
                dns: Vec::new(),
            });

            // ACT
            let described = describe_v6(notice);

            // ASSERT
            assert_eq!(
                described,
                Some((
                    Level::Info,
                    "IPv6 slaac none routers none dns none".to_owned()
                ))
            );
        }

        #[test]
        fn long_slaac_lists_are_capped() {
            // ARRANGE
            let notice = V6Notice::Configured(V6State::Slaac {
                addresses: (1..=5)
                    .map(|last| (Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, last), 64))
                    .collect(),
                routers: (1..=6)
                    .map(|last| Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, last))
                    .collect(),
                dns: (1..=4)
                    .map(|last| Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, last))
                    .collect(),
            });

            // ACT
            let described = describe_v6(notice);

            // ASSERT
            assert_eq!(
                described,
                Some((
                    Level::Info,
                    "IPv6 slaac ::1/64, ::2/64, ::3/64, ::4/64, +1 more routers ::1, ::2, ::3, ::4, +2 more dns ::1, ::2, ::3, ::4"
                        .to_owned()
                ))
            );
        }

        #[test]
        fn bound_state_is_not_described() {
            // ARRANGE
            let notice = V6Notice::State(provision::State::Bound);

            // ACT
            let described = describe_v6(notice);

            // ASSERT
            assert_eq!(described, None);
        }

        #[test]
        fn acquiring_state_is_described_at_info() {
            // ARRANGE
            let notice = V6Notice::State(provision::State::Acquiring);

            // ACT
            let described = describe_v6(notice);

            // ASSERT
            assert_eq!(
                described,
                Some((Level::Info, "IPv6 state acquiring".to_owned()))
            );
        }

        #[test]
        fn warning_is_described_at_warn() {
            // ARRANGE
            let notice = V6Notice::Warning("SLAAC: no router advertisement".to_owned());

            // ACT
            let described = describe_v6(notice);

            // ASSERT
            assert_eq!(
                described,
                Some((
                    Level::Warn,
                    "IPv6 SLAAC: no router advertisement".to_owned()
                ))
            );
        }
    }
}
