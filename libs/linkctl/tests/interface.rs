//! Integration tests for the family-neutral interface controller.

mod common;

#[cfg(test)]
mod tests {
    use core::time::Duration;

    use linkctl::interface::{Command, Interface, Snapshot};
    #[cfg(feature = "ipv4")]
    use linkctl::interface::{Handle, Notice, V4Notice};
    use linkctl::provision;
    use netdev::interface::Ethernet;
    use netlink::link::ifname::Name;
    use netlink::link::state::State;
    #[cfg(feature = "ipv4")]
    use tokio::sync::mpsc;
    use tokio::sync::watch;

    use crate::common::MockNetlinkOps;

    fn ethernet(mock: &MockNetlinkOps) -> Ethernet {
        let index = mock.add_link("eth0", [0xAA; 6], true);

        Ethernet::new(
            Name::new("eth0").expect("valid name"),
            index,
            [0xAA; 6],
            State::Up,
        )
    }

    #[cfg(feature = "ipv4")]
    async fn next_v4_notice(rx: &mut mpsc::Receiver<V4Notice>) -> V4Notice {
        tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timed out waiting for notice")
            .expect("notice channel closed")
    }

    #[cfg(feature = "ipv4")]
    fn drain_v4_configured(rx: &mut mpsc::Receiver<V4Notice>) -> usize {
        let mut configured = 0_usize;
        while let Ok(notice) = rx.try_recv() {
            configured =
                configured.saturating_add(usize::from(matches!(notice, Notice::Configured(_))));
        }

        configured
    }

    #[cfg(feature = "ipv4")]
    fn drain_v4(rx: &mut mpsc::Receiver<V4Notice>) -> impl Iterator<Item = V4Notice> + '_ {
        core::iter::from_fn(move || rx.try_recv().ok())
    }

    #[cfg(feature = "ipv4")]
    fn take_v4_configured(rx: &mut mpsc::Receiver<V4Notice>) -> Option<linkctl::view::V4State> {
        drain_v4(rx).find_map(|notice| match notice {
            Notice::Configured(view) => Some(view),
            Notice::State(_) | Notice::Warning(_) => None,
        })
    }

    #[cfg(feature = "ipv4")]
    async fn wait_for_v4_address(
        rx: &mut watch::Receiver<Snapshot>,
        address: core::net::Ipv4Addr,
    ) -> Snapshot {
        tokio::time::timeout(
            Duration::from_secs(1),
            rx.wait_for(|snapshot| {
                matches!(
                    &snapshot.v4,
                    Some(linkctl::view::V4State::Static(config)) if config.address == address
                )
            }),
        )
        .await
        .expect("timed out waiting for snapshot")
        .expect("snapshot channel closed")
        .clone()
    }

    #[cfg(feature = "ipv4")]
    async fn wait_for_v4(rx: &mut watch::Receiver<Snapshot>, state: provision::State) -> Snapshot {
        tokio::time::timeout(
            Duration::from_secs(1),
            rx.wait_for(|snapshot| snapshot.v4_state == state),
        )
        .await
        .expect("timed out waiting for snapshot")
        .expect("snapshot channel closed")
        .clone()
    }

    #[cfg(feature = "ipv6")]
    async fn wait_for_v6(rx: &mut watch::Receiver<Snapshot>, state: provision::State) -> Snapshot {
        tokio::time::timeout(
            Duration::from_secs(1),
            rx.wait_for(|snapshot| snapshot.v6_state == state),
        )
        .await
        .expect("timed out waiting for snapshot")
        .expect("snapshot channel closed")
        .clone()
    }

    #[cfg(feature = "ipv4")]
    #[tokio::test]
    async fn configure_static_v4_applies_address_and_route() {
        use core::net::Ipv4Addr;

        use linkctl::view::V4State;

        // ARRANGE
        let mock = MockNetlinkOps::new();
        let link = ethernet(&mock);
        let index = link.index;
        let address = Ipv4Addr::new(10, 0, 0, 2);
        let gateway = Ipv4Addr::new(10, 0, 0, 1);
        let config = provision::v4::Config {
            addresses: vec![(address, 24)],
            gateway: Some(gateway),
            dns: Vec::new(),
            dhcp: false,
        };
        let (mut handle, _notices) = Interface::spawn(mock.clone(), link);
        let command = Command::Configure {
            v4: Some(config),
            #[cfg(feature = "ipv6")]
            v6: None,
        };

        // ACT
        handle
            .command_tx
            .send(command)
            .await
            .expect("send configure");
        let snapshot = wait_for_v4(&mut handle.snapshot_rx, provision::State::Bound).await;

        // ASSERT
        assert!(matches!(
            snapshot.v4,
            Some(V4State::Static(config)) if config.address == address && config.prefix_len == 24
        ));
        assert!(mock.lock().default_routes_v4.contains(&gateway));
        assert!(
            mock.lock()
                .ipv4_addrs
                .get(&index)
                .is_some_and(|addrs| addrs.contains(&(address, 24)))
        );
    }

    #[cfg(feature = "ipv6")]
    #[tokio::test]
    async fn configure_static_v6_applies_address_and_route() {
        use core::net::Ipv6Addr;

        use linkctl::view::V6State;

        // ARRANGE
        let mock = MockNetlinkOps::new();
        let link = ethernet(&mock);
        let index = link.index;
        let address = Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 2);
        let gateway = Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 1);
        let config = provision::v6::Config {
            addresses: vec![(address, 64)],
            gateway: Some(gateway),
            dns: Vec::new(),
            autoconf: false,
        };
        let (mut handle, _notices) = Interface::spawn(mock.clone(), link);
        let command = Command::Configure {
            #[cfg(feature = "ipv4")]
            v4: None,
            v6: Some(config),
        };

        // ACT
        handle
            .command_tx
            .send(command)
            .await
            .expect("send configure");
        let snapshot = wait_for_v6(&mut handle.snapshot_rx, provision::State::Bound).await;

        // ASSERT
        assert!(matches!(
            snapshot.v6,
            Some(V6State::Static(config)) if config.address == address && config.prefix_len == 64
        ));
        assert!(mock.lock().default_routes_v6.contains(&gateway));
        assert!(
            mock.lock()
                .ipv6_addrs
                .get(&index)
                .is_some_and(|addrs| addrs.contains(&(address, 64)))
        );
    }

    #[cfg(feature = "ipv4")]
    async fn configured_v4() -> (MockNetlinkOps, Handle) {
        use core::net::Ipv4Addr;

        let mock = MockNetlinkOps::new();
        let link = ethernet(&mock);
        let (mut handle, _notices) = Interface::spawn(mock.clone(), link);
        let config = provision::v4::Config {
            addresses: vec![(Ipv4Addr::new(10, 0, 0, 2), 24)],
            gateway: None,
            dns: Vec::new(),
            dhcp: false,
        };
        let command = Command::Configure {
            v4: Some(config),
            #[cfg(feature = "ipv6")]
            v6: None,
        };
        handle
            .command_tx
            .send(command)
            .await
            .expect("send configure");
        wait_for_v4(&mut handle.snapshot_rx, provision::State::Bound).await;

        (mock, handle)
    }

    #[cfg(feature = "ipv4")]
    #[tokio::test]
    async fn link_down_keeps_view_and_degrades() {
        // ARRANGE
        let (_mock, mut handle) = configured_v4().await;

        // ACT
        handle
            .command_tx
            .send(Command::LinkDown)
            .await
            .expect("send link down");
        let snapshot = wait_for_v4(&mut handle.snapshot_rx, provision::State::Degraded).await;

        // ASSERT
        assert!(snapshot.v4.is_some());
        assert_eq!(snapshot.link, State::Down);
    }

    #[cfg(feature = "ipv4")]
    #[tokio::test]
    async fn bound_emits_configured_once() {
        use core::net::Ipv4Addr;

        // ARRANGE
        let mock = MockNetlinkOps::new();
        let link = ethernet(&mock);
        let config = provision::v4::Config {
            addresses: vec![(Ipv4Addr::new(10, 0, 0, 2), 24)],
            gateway: None,
            dns: Vec::new(),
            dhcp: false,
        };
        let (mut handle, mut notices) = Interface::spawn(mock.clone(), link);
        handle
            .command_tx
            .send(Command::Configure {
                v4: Some(config),
                #[cfg(feature = "ipv6")]
                v6: None,
            })
            .await
            .expect("send configure");
        wait_for_v4(&mut handle.snapshot_rx, provision::State::Bound).await;
        let mut configured = drain_v4_configured(&mut notices.v4);

        // ACT
        handle
            .command_tx
            .send(Command::LinkDown)
            .await
            .expect("send link down");
        wait_for_v4(&mut handle.snapshot_rx, provision::State::Degraded).await;
        let _drained = drain_v4_configured(&mut notices.v4);
        handle
            .command_tx
            .send(Command::LinkUp)
            .await
            .expect("send link up");
        wait_for_v4(&mut handle.snapshot_rx, provision::State::Bound).await;
        configured = configured.saturating_add(drain_v4_configured(&mut notices.v4));

        // ASSERT
        assert_eq!(configured, 1);
    }

    #[cfg(feature = "ipv4")]
    #[tokio::test]
    async fn reconfigure_emits_configured_again() {
        use core::net::Ipv4Addr;

        use linkctl::view::V4State;

        // ARRANGE
        let mock = MockNetlinkOps::new();
        let link = ethernet(&mock);
        let first = provision::v4::Config {
            addresses: vec![(Ipv4Addr::new(10, 0, 0, 2), 24)],
            gateway: None,
            dns: Vec::new(),
            dhcp: false,
        };
        let second = provision::v4::Config {
            addresses: vec![(Ipv4Addr::new(10, 0, 0, 3), 24)],
            gateway: None,
            dns: Vec::new(),
            dhcp: false,
        };
        let (mut handle, mut notices) = Interface::spawn(mock, link);
        handle
            .command_tx
            .send(Command::Configure {
                v4: Some(first),
                #[cfg(feature = "ipv6")]
                v6: None,
            })
            .await
            .expect("send configure");
        wait_for_v4(&mut handle.snapshot_rx, provision::State::Bound).await;
        let first_view = take_v4_configured(&mut notices.v4).expect("first configured notice");

        // ACT
        handle
            .command_tx
            .send(Command::Configure {
                v4: Some(second),
                #[cfg(feature = "ipv6")]
                v6: None,
            })
            .await
            .expect("send configure");
        wait_for_v4_address(&mut handle.snapshot_rx, Ipv4Addr::new(10, 0, 0, 3)).await;
        let second_view = take_v4_configured(&mut notices.v4).expect("second configured notice");

        // ASSERT
        assert!(matches!(
            first_view,
            V4State::Static(config) if config.address == Ipv4Addr::new(10, 0, 0, 2)
        ));
        assert!(matches!(
            second_view,
            V4State::Static(config) if config.address == Ipv4Addr::new(10, 0, 0, 3)
        ));
    }

    #[cfg(feature = "ipv4")]
    #[tokio::test]
    async fn consecutive_failures_emit_single_degraded() {
        // ARRANGE
        let mock = MockNetlinkOps::new();
        let link = Ethernet::new(
            Name::new("ghost").expect("valid name"),
            1,
            [0xAA; 6],
            State::Up,
        );
        let config = provision::v4::Config {
            addresses: Vec::new(),
            gateway: None,
            dns: Vec::new(),
            dhcp: true,
        };
        let (mut handle, mut notices) = Interface::spawn(mock, link);
        handle
            .command_tx
            .send(Command::Configure {
                v4: Some(config.clone()),
                #[cfg(feature = "ipv6")]
                v6: None,
            })
            .await
            .expect("send configure");
        wait_for_v4(&mut handle.snapshot_rx, provision::State::Degraded).await;
        let first_warning = next_v4_notice(&mut notices.v4).await;
        let first_state = next_v4_notice(&mut notices.v4).await;

        // ACT
        handle
            .command_tx
            .send(Command::Configure {
                v4: Some(config),
                #[cfg(feature = "ipv6")]
                v6: None,
            })
            .await
            .expect("send configure");
        let second_warning = next_v4_notice(&mut notices.v4).await;
        tokio::task::yield_now().await;
        let leftovers: Vec<V4Notice> = drain_v4(&mut notices.v4).collect();

        // ASSERT
        assert!(matches!(first_warning, Notice::Warning(_)));
        assert!(matches!(
            first_state,
            Notice::State(provision::State::Degraded)
        ));
        assert!(matches!(second_warning, Notice::Warning(_)));
        assert!(
            !leftovers
                .iter()
                .any(|notice| matches!(notice, Notice::State(provision::State::Degraded)))
        );
    }

    #[cfg(feature = "ipv4")]
    #[tokio::test]
    async fn refresh_failure_emits_warning() {
        use core::net::Ipv4Addr;

        // ARRANGE
        let mock = MockNetlinkOps::new();
        let link = Ethernet::new(
            Name::new("ghost").expect("valid name"),
            1,
            [0xAA; 6],
            State::Up,
        );
        let config = provision::v4::Config {
            addresses: vec![(Ipv4Addr::new(10, 0, 0, 2), 24)],
            gateway: None,
            dns: Vec::new(),
            dhcp: false,
        };
        let (handle, mut notices) = Interface::spawn(mock, link);

        // ACT
        handle
            .command_tx
            .send(Command::Configure {
                v4: Some(config),
                #[cfg(feature = "ipv6")]
                v6: None,
            })
            .await
            .expect("send configure");
        let notice = next_v4_notice(&mut notices.v4).await;

        // ASSERT
        assert!(matches!(notice, Notice::Warning(_)));
    }

    #[cfg(feature = "ipv4")]
    #[tokio::test]
    async fn shutdown_stops_family() {
        // ARRANGE
        let (_mock, mut handle) = configured_v4().await;

        // ACT
        handle
            .command_tx
            .send(Command::Shutdown)
            .await
            .expect("send shutdown");
        let snapshot = wait_for_v4(&mut handle.snapshot_rx, provision::State::Stopped).await;

        // ASSERT
        assert!(snapshot.v4.is_none());
        assert_eq!(snapshot.v4_state, provision::State::Stopped);
    }
}
