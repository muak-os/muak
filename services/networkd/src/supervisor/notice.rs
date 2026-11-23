//! Interface controller notices translated into kernel log lines.

use core::fmt::Display;

use addrconf::State;
use linkctl::interface::Notice;
#[cfg(feature = "ipv4")]
use linkctl::interface::V4Notice;
#[cfg(feature = "ipv6")]
use linkctl::interface::V6Notice;
use netlink::link::ifname::Name;
use tokio::sync::mpsc;

// Kernel log priority a described notice maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Info,
    Warn,
}

/// Logs IPv4 notices until the controller drops its sender.
#[cfg(feature = "ipv4")]
pub(super) async fn log_v4(name: Name, rx: mpsc::Receiver<V4Notice>) {
    log(name, "IPv4", rx).await;
}

/// Logs IPv6 notices until the controller drops its sender.
#[cfg(feature = "ipv6")]
pub(super) async fn log_v6(name: Name, rx: mpsc::Receiver<V6Notice>) {
    log(name, "IPv6", rx).await;
}

async fn log<V: Display>(name: Name, family: &'static str, mut rx: mpsc::Receiver<Notice<V>>) {
    while let Some(notice) = rx.recv().await {
        let Some((level, text)) = describe(notice, family) else {
            continue;
        };
        match level {
            Level::Info => kmsg::info!("{name}: {text}"),
            Level::Warn => kmsg::warn!("{name}: {text}"),
        }
    }
}

fn describe<V: Display>(notice: Notice<V>, family: &'static str) -> Option<(Level, String)> {
    match notice {
        Notice::State(State::Bound) => None,
        Notice::State(state) => Some((Level::Info, format!("{family} state {}", phase(state)))),
        Notice::Configured(state) => Some((Level::Info, format!("{family} {state}"))),
        Notice::Warning(warning) => Some((Level::Warn, format!("{family} {warning}"))),
    }
}

const fn phase(state: State) -> &'static str {
    match state {
        State::Acquiring => "acquiring",
        State::Bound => "bound",
        State::Degraded => "degraded",
        State::Stopped => "stopped",
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "ipv4")]
    mod v4 {
        use core::net::Ipv4Addr;

        use addrconf::State;
        use addrconf::fixed::StaticView;
        use linkctl::interface::V4Notice;
        use linkctl::view::V4State;

        use super::super::{Level, describe};

        fn configured() -> V4Notice {
            V4Notice::Configured(V4State::Static(StaticView {
                address: Ipv4Addr::new(192, 168, 1, 10),
                prefix_len: 24,
                gateway: None,
                dns: Vec::new(),
            }))
        }

        #[test]
        fn configured_state_is_described_at_info() {
            // ARRANGE
            let notice = configured();

            // ACT
            let described = describe(notice, "IPv4");

            // ASSERT
            let (level, text) = described.expect("configured state must be described");
            assert_eq!(level, Level::Info);
            assert!(text.starts_with("IPv4 "));
        }

        #[test]
        fn bound_state_is_not_described() {
            // ARRANGE
            let notice = V4Notice::State(State::Bound);

            // ACT
            let described = describe(notice, "IPv4");

            // ASSERT
            assert_eq!(described, None);
        }

        #[test]
        fn degraded_state_is_described_at_info() {
            // ARRANGE
            let notice = V4Notice::State(State::Degraded);

            // ACT
            let described = describe(notice, "IPv4");

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
            let described = describe(notice, "IPv4");

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

        use addrconf::State;
        use addrconf::fixed::StaticView;
        use linkctl::interface::V6Notice;
        use linkctl::view::V6State;

        use super::super::{Level, describe};

        fn configured() -> V6Notice {
            V6Notice::Configured(V6State::Static(StaticView {
                address: Ipv6Addr::LOCALHOST,
                prefix_len: 64,
                gateway: Some(Ipv6Addr::UNSPECIFIED),
                dns: Vec::new(),
            }))
        }

        #[test]
        fn configured_state_is_described_at_info() {
            // ARRANGE
            let notice = configured();

            // ACT
            let described = describe(notice, "IPv6");

            // ASSERT
            let (level, text) = described.expect("configured state must be described");
            assert_eq!(level, Level::Info);
            assert!(text.starts_with("IPv6 "));
        }

        #[test]
        fn bound_state_is_not_described() {
            // ARRANGE
            let notice = V6Notice::State(State::Bound);

            // ACT
            let described = describe(notice, "IPv6");

            // ASSERT
            assert_eq!(described, None);
        }

        #[test]
        fn acquiring_state_is_described_at_info() {
            // ARRANGE
            let notice = V6Notice::State(State::Acquiring);

            // ACT
            let described = describe(notice, "IPv6");

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
            let described = describe(notice, "IPv6");

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
