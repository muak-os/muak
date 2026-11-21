//! Integration tests for the family-neutral interface controller.

mod common;

#[cfg(test)]
mod tests {
    use core::time::Duration;

    use netdev::interface::Ethernet;
    use netlink::link::ifname::Name;
    use netlink::link::state::State;
    #[cfg(feature = "ipv4")]
    use netstack::interface::Handle;
    use netstack::interface::{Command, Interface, Snapshot};
    use netstack::provision;
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

        use netstack::view::V4State;

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
        let mut handle = Interface::spawn(mock.clone(), link);
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

        use netstack::view::V6State;

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
        let mut handle = Interface::spawn(mock.clone(), link);
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
        let mut handle = Interface::spawn(mock.clone(), link);
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
