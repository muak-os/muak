//! Startup readiness tests over the deterministic mock runtime.

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::io;
    use std::time::{Duration, Instant, SystemTime};

    use net_runtime::mock::Mock;
    use net_runtime::{Events, Runtime};
    use networkd::service::{Readiness, Service};

    /// Event port that never produces an event, so waiting must rely on the timer.
    struct Paused;

    impl Events for Paused {
        fn next(&self) -> impl Future<Output = io::Result<netmodel::LinkEvent>> + Send {
            std::future::pending()
        }
    }

    /// Mock runtime with a blocking event port so readiness waits are deterministic.
    struct Sandbox {
        mock: Mock,
    }

    impl Runtime for Sandbox {
        type Address = Mock;
        type Clock = Mock;
        type Entropy = Mock;
        type Events = Paused;
        type Icmpv6 = Mock;
        type Link = Mock;
        type Packet = Mock;
        type Route = Mock;
        type Udp = Mock;

        fn address(&self) -> &Mock {
            &self.mock
        }

        fn clock(&self) -> &Mock {
            &self.mock
        }

        fn entropy(&self) -> &Mock {
            &self.mock
        }

        fn events(&self) -> &Paused {
            static PAUSED: Paused = Paused;
            &PAUSED
        }

        fn icmpv6(&self) -> &Mock {
            &self.mock
        }

        fn link(&self) -> &Mock {
            &self.mock
        }

        fn packet(&self) -> &Mock {
            &self.mock
        }

        fn route(&self) -> &Mock {
            &self.mock
        }

        fn udp(&self) -> &Mock {
            &self.mock
        }
    }

    fn service(state: netmodel::LinkState) -> Service<Sandbox> {
        let mock = Mock::new(Instant::now(), SystemTime::UNIX_EPOCH);
        mock.set_link(netmodel::Link {
            name: netmodel::Name::new("eth0").expect("valid name"),
            index: 2,
            mac: netmodel::Mac::default(),
            admin: state.admin(),
            carrier: state.carrier(),
            is_virtual: false,
            master: None,
        });

        Service::new(Sandbox { mock }, config())
    }

    fn config() -> net_supervisor::Config {
        net_supervisor::Config {
            ipv6: false,
            dns: Vec::new(),
            interfaces: vec![net_supervisor::InterfaceConfig {
                name: String::from("eth0"),
                kind: net_supervisor::InterfaceKind::Ethernet,
                ipv4: Some(netmodel::Ipv4Config {
                    addresses: vec![(core::net::Ipv4Addr::new(10, 0, 0, 2), 24)],
                    gateway: None,
                    dns: Vec::new(),
                    dhcp: false,
                }),
                ipv6: None,
                bridge: None,
            }],
        }
    }

    #[tokio::test]
    async fn carrier_reaches_ready() {
        // ARRANGE
        let mut service = service(netmodel::LinkState::Up);

        // ACT
        service.start().await.expect("start");
        let readiness = service
            .wait_ready(Duration::from_secs(1))
            .await
            .expect("wait ready");

        // ASSERT
        assert_eq!(readiness, Readiness::Ready);
    }

    #[tokio::test]
    async fn missing_carrier_degrades_after_the_timeout() {
        // ARRANGE
        let mut service = service(netmodel::LinkState::NoCarrier);

        // ACT
        service.start().await.expect("start");
        let readiness = service
            .wait_ready(Duration::from_millis(10))
            .await
            .expect("wait ready");

        // ASSERT
        assert_eq!(readiness, Readiness::Degraded);
    }

    #[tokio::test]
    async fn elapsed_timeout_degrades_without_blocking() {
        // ARRANGE
        let mut service = service(netmodel::LinkState::NoCarrier);

        // ACT
        service.start().await.expect("start");
        let readiness = service
            .wait_ready(Duration::ZERO)
            .await
            .expect("wait ready");

        // ASSERT
        assert_eq!(readiness, Readiness::Degraded);
    }
}
