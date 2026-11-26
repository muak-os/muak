//! Tokio backend construction, clock, entropy, and datagram tests.

#![cfg(feature = "tokio")]

#[cfg(test)]
mod backend {
    use std::io;
    use std::time::{Duration, SystemTime};

    use ip_method::{SocketKind, SocketSlot, SocketSpec};
    use net_runtime_linux::Tokio;
    use netmodel::Name;
    use tokio::time::timeout;

    fn spec(kind: SocketKind, bind: std::net::SocketAddr) -> SocketSpec {
        SocketSpec {
            slot: SocketSlot::Primary,
            kind,
            link: Name::new("lo").expect("loopback name is valid"),
            bind: Some(bind),
            peer: Some(bind),
        }
    }

    fn free_loopback_address() -> std::net::SocketAddr {
        let probe = std::net::UdpSocket::bind("127.0.0.1:0").expect("probe socket");

        probe.local_addr().expect("probe address")
    }

    #[tokio::test]
    async fn accessors_return_the_backend() {
        // ARRANGE
        let runtime = Tokio::new().expect("runtime should open");
        let this = std::ptr::from_ref(&runtime);

        // ACT
        let address = std::ptr::from_ref(net_runtime::Runtime::address(&runtime));
        let clock = std::ptr::from_ref(net_runtime::Runtime::clock(&runtime));
        let entropy = std::ptr::from_ref(net_runtime::Runtime::entropy(&runtime));
        let events = std::ptr::from_ref(net_runtime::Runtime::events(&runtime));
        let icmpv6 = std::ptr::from_ref(net_runtime::Runtime::icmpv6(&runtime));
        let link = std::ptr::from_ref(net_runtime::Runtime::link(&runtime));
        let packet = std::ptr::from_ref(net_runtime::Runtime::packet(&runtime));
        let route = std::ptr::from_ref(net_runtime::Runtime::route(&runtime));
        let udp = std::ptr::from_ref(net_runtime::Runtime::udp(&runtime));

        // ASSERT
        assert!(std::ptr::eq(address, this));
        assert!(std::ptr::eq(clock, this));
        assert!(std::ptr::eq(entropy, this));
        assert!(std::ptr::eq(events, this));
        assert!(std::ptr::eq(icmpv6, this));
        assert!(std::ptr::eq(link, this));
        assert!(std::ptr::eq(packet, this));
        assert!(std::ptr::eq(route, this));
        assert!(std::ptr::eq(udp, this));
    }

    #[tokio::test]
    async fn clock_reports_monotonic_and_wall_time() {
        // ARRANGE
        let runtime = Tokio::new().expect("runtime should open");

        // ACT
        let first = net_runtime::Clock::now(&runtime);
        let second = net_runtime::Clock::now(&runtime);
        let wall = net_runtime::Clock::wall_now(&runtime);

        // ASSERT
        assert!(second >= first);
        assert!(wall > SystemTime::UNIX_EPOCH);
    }

    #[tokio::test]
    async fn entropy_fills_distinct_buffers() {
        // ARRANGE
        let runtime = Tokio::new().expect("runtime should open");
        let mut first = [0_u8; 32];
        let mut second = [0_u8; 32];

        // ACT
        net_runtime::Entropy::fill(&runtime, &mut first);
        net_runtime::Entropy::fill(&runtime, &mut second);

        // ASSERT
        assert_ne!(first, second);
        assert!(first.iter().any(|byte| *byte != 0));
    }

    #[tokio::test]
    async fn udp_loopback_send_and_receive() {
        // ARRANGE
        let runtime = Tokio::new().expect("runtime should open");
        let address = free_loopback_address();
        let spec = spec(SocketKind::Udp, address);
        let id = net_runtime::Udp::open(&runtime, &spec)
            .await
            .expect("udp socket should open");

        // ACT
        let sent = net_runtime::Udp::send(&runtime, id, b"ping")
            .await
            .expect("udp send should succeed");
        let mut buffer = [0_u8; 16];
        let received = timeout(
            Duration::from_secs(2),
            net_runtime::Udp::recv(&runtime, id, &mut buffer),
        )
        .await
        .expect("udp receive should not time out")
        .expect("udp receive should succeed");
        net_runtime::Udp::close(&runtime, id)
            .await
            .expect("udp socket should close");

        // ASSERT
        assert_eq!(sent, 4);
        assert_eq!(&buffer[..received], b"ping");
    }

    #[tokio::test]
    async fn link_reports_the_loopback() {
        // ARRANGE
        let runtime = Tokio::new().expect("runtime should open");
        let name = Name::new("lo").expect("loopback name is valid");

        // ACT
        let link = net_runtime::Link::link(&runtime, &name)
            .await
            .expect("link query should succeed")
            .expect("loopback should exist");

        // ASSERT
        assert!(link.index > 0);
        assert_eq!(link.name, name);
    }

    #[tokio::test]
    async fn link_skips_a_missing_link() {
        // ARRANGE
        let runtime = Tokio::new().expect("runtime should open");
        let name = Name::new("nomissing0").expect("missing name is valid");

        // ACT
        let link = net_runtime::Link::link(&runtime, &name)
            .await
            .expect("link query should succeed");

        // ASSERT
        assert!(link.is_none());
    }

    #[tokio::test]
    async fn packet_rejects_a_datagram_specification() {
        // ARRANGE
        let runtime = Tokio::new().expect("runtime should open");
        let spec = spec(SocketKind::Udp, free_loopback_address());

        // ACT
        let result = net_runtime::Packet::open(&runtime, &spec).await;

        // ASSERT
        assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::InvalidInput));
    }
}
