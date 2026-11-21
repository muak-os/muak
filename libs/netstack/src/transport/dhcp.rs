//! Production `DHCPv4` transport and exchange backed by kernel sockets.

use core::future::Future;
use core::net::Ipv4Addr;

use dhcp_client::datagram::Datagram;
use dhcp_client::lease::Lease;
use dhcp_client::{Failure, MAX_FRAME_LEN, MAX_MESSAGE_LEN, Scratch};
use dhcp_proto::DHCP_CLIENT_PORT;

use crate::provision::v4;

/// Creates the sockets an exchange needs.
pub trait Transport: Send + Sync {
    /// Raw broadcast transport bound to an interface.
    type Raw: Datagram;
    /// Unicast transport bound to an interface and source address.
    type Unicast: Datagram;

    /// Opens the raw broadcast transport bound to `interface`.
    fn raw(&self, interface: &str) -> impl Future<Output = std::io::Result<Self::Raw>> + Send;

    /// Opens the unicast transport bound to `interface` with `src`.
    fn unicast(
        &self,
        interface: &str,
        src: Ipv4Addr,
    ) -> impl Future<Output = std::io::Result<Self::Unicast>> + Send;
}

/// Transport backed by the kernel.
pub struct System;

impl Transport for System {
    type Raw = packet::v4::Socket;
    type Unicast = tokio::net::UdpSocket;

    fn raw(&self, interface: &str) -> impl Future<Output = std::io::Result<Self::Raw>> + Send {
        std::future::ready(packet::v4::Socket::open(interface).map_err(std::io::Error::other))
    }

    async fn unicast(&self, interface: &str, src: Ipv4Addr) -> std::io::Result<Self::Unicast> {
        let socket = tokio::net::UdpSocket::bind((src, DHCP_CLIENT_PORT)).await?;
        packet::socket::bind_device(&socket, interface).map_err(std::io::Error::other)?;

        Ok(socket)
    }
}

/// `DHCPv4` exchange over a [`Transport`].
pub struct Exchange<T: Transport> {
    transport: T,
}

impl Exchange<System> {
    /// Opens the exchange for `interface`.
    ///
    /// # Errors
    ///
    /// Returns an error when the raw socket cannot be opened on `interface`.
    pub async fn open(interface: &str) -> dhcp_client::Result<Self> {
        let transport = System;
        // Probe once so missing interfaces and permissions fail setup instead of acquisition;
        // each exchange opens the socket it needs.
        transport.raw(interface).await.map_err(Failure::Io)?;

        Ok(Self::new(transport))
    }
}

impl<T: Transport> Exchange<T> {
    /// Pairs an exchange with its transport.
    #[must_use = "the exchange must be driven by the DHCP method"]
    pub const fn new(transport: T) -> Self {
        Self { transport }
    }
}

impl<T: Transport> v4::Exchange for Exchange<T> {
    async fn acquire(&self, interface: &str, mac: [u8; 6]) -> dhcp_client::Result<Lease> {
        let socket = self.transport.raw(interface).await.map_err(Failure::Io)?;
        let mut msg = [0_u8; MAX_MESSAGE_LEN];
        let mut frame = [0_u8; MAX_FRAME_LEN];

        dhcp_client::exchange::acquire(&socket, mac, Scratch::new(&mut msg, &mut frame)).await
    }

    async fn rebind(
        &self,
        interface: &str,
        mac: [u8; 6],
        server: Ipv4Addr,
        assigned: Ipv4Addr,
    ) -> dhcp_client::Result<Lease> {
        let socket = self.transport.raw(interface).await.map_err(Failure::Io)?;
        let mut msg = [0_u8; MAX_MESSAGE_LEN];
        let mut frame = [0_u8; MAX_FRAME_LEN];

        dhcp_client::exchange::rebind(
            &socket,
            mac,
            server,
            assigned,
            Scratch::new(&mut msg, &mut frame),
        )
        .await
    }

    async fn renew(
        &self,
        interface: &str,
        mac: [u8; 6],
        server: Ipv4Addr,
        assigned: Ipv4Addr,
    ) -> dhcp_client::Result<Lease> {
        let socket = self
            .transport
            .unicast(interface, assigned)
            .await
            .map_err(Failure::Io)?;
        let mut msg = [0_u8; MAX_MESSAGE_LEN];
        let mut frame = [0_u8; MAX_FRAME_LEN];

        dhcp_client::exchange::renew(
            &socket,
            mac,
            server,
            assigned,
            Scratch::new(&mut msg, &mut frame),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use core::future::Future;
    use core::net::Ipv4Addr;
    use std::sync::Mutex;

    use dhcp_client::Failure;
    use dhcp_client::datagram::{Datagram, Received};
    use ip::v4::Addr;

    use super::{Exchange, Transport};
    use crate::provision::v4::Exchange as _;

    const INTERFACE: &str = "eth0";
    const MAC: [u8; 6] = [0xAA; 6];
    const SERVER: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);
    const ASSIGNED: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 2);

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Kind {
        Raw,
        Unicast,
    }

    struct MockDatagram<'a> {
        kind: Kind,
        used: &'a Mutex<Vec<Kind>>,
    }

    impl Datagram for MockDatagram<'_> {
        fn recv<'a>(
            &'a self,
            _frame: &'a mut [u8],
        ) -> impl Future<Output = dhcp_client::Result<Option<Received<'a>>>> + Send + 'a {
            self.used.lock().expect("use log").push(self.kind);

            std::future::ready(Err(Failure::Timeout))
        }

        fn send<'a>(
            &'a self,
            _payload: &'a [u8],
            _src: Addr,
            _dst: Addr,
            _frame: &'a mut [u8],
        ) -> impl Future<Output = dhcp_client::Result<()>> + Send + 'a {
            std::future::ready(Ok(()))
        }
    }

    struct MockTransport<'a> {
        used: &'a Mutex<Vec<Kind>>,
    }

    impl<'a> Transport for MockTransport<'a> {
        type Raw = MockDatagram<'a>;
        type Unicast = MockDatagram<'a>;

        fn raw(&self, _interface: &str) -> impl Future<Output = std::io::Result<Self::Raw>> + Send {
            std::future::ready(Ok(MockDatagram {
                kind: Kind::Raw,
                used: self.used,
            }))
        }

        fn unicast(
            &self,
            _interface: &str,
            _src: Ipv4Addr,
        ) -> impl Future<Output = std::io::Result<Self::Unicast>> + Send {
            std::future::ready(Ok(MockDatagram {
                kind: Kind::Unicast,
                used: self.used,
            }))
        }
    }

    struct Failing;

    impl Transport for Failing {
        type Raw = MockDatagram<'static>;
        type Unicast = MockDatagram<'static>;

        fn raw(&self, _interface: &str) -> impl Future<Output = std::io::Result<Self::Raw>> + Send {
            std::future::ready(Err(std::io::Error::other("open failed")))
        }

        fn unicast(
            &self,
            _interface: &str,
            _src: Ipv4Addr,
        ) -> impl Future<Output = std::io::Result<Self::Unicast>> + Send {
            std::future::ready(Err(std::io::Error::other("open failed")))
        }
    }

    fn exchange(used: &Mutex<Vec<Kind>>) -> Exchange<MockTransport<'_>> {
        Exchange::new(MockTransport { used })
    }

    #[tokio::test]
    async fn acquire_uses_the_raw_transport() {
        // ARRANGE
        let used = Mutex::new(Vec::new());

        // ACT
        let result = exchange(&used).acquire(INTERFACE, MAC).await;

        // ASSERT
        assert!(matches!(result, Err(Failure::Timeout)));
        assert_eq!(*used.lock().expect("use log"), [Kind::Raw]);
    }

    #[tokio::test]
    async fn renew_uses_the_unicast_transport() {
        // ARRANGE
        let used = Mutex::new(Vec::new());

        // ACT
        let result = exchange(&used)
            .renew(INTERFACE, MAC, SERVER, ASSIGNED)
            .await;

        // ASSERT
        assert!(matches!(result, Err(Failure::Timeout)));
        assert_eq!(*used.lock().expect("use log"), [Kind::Unicast]);
    }

    #[tokio::test]
    async fn rebind_uses_the_raw_transport() {
        // ARRANGE
        let used = Mutex::new(Vec::new());

        // ACT
        let result = exchange(&used)
            .rebind(INTERFACE, MAC, SERVER, ASSIGNED)
            .await;

        // ASSERT
        assert!(matches!(result, Err(Failure::Timeout)));
        assert_eq!(*used.lock().expect("use log"), [Kind::Raw]);
    }

    #[tokio::test]
    async fn transport_error_maps_to_io() {
        // ARRANGE
        let exchange = Exchange::new(Failing);

        // ACT
        let result = exchange.acquire(INTERFACE, MAC).await;

        // ASSERT
        assert!(matches!(result, Err(Failure::Io(_))));
    }
}
