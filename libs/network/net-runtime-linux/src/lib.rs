//! Linux network runtime backend with optional tokio support.

#![warn(missing_docs)]

#[cfg(feature = "tokio")]
mod address;
#[cfg(feature = "tokio")]
mod clock;
#[cfg(feature = "tokio")]
mod control;
#[cfg(feature = "tokio")]
mod entropy;
#[cfg(feature = "tokio")]
mod event;
#[cfg(feature = "tokio")]
mod icmpv6;
#[cfg(feature = "tokio")]
mod link;
#[cfg(feature = "tokio")]
mod packet;
#[cfg(feature = "tokio")]
mod route;
#[cfg(feature = "tokio")]
mod socket;
#[cfg(feature = "tokio")]
mod udp;

#[cfg(feature = "tokio")]
use std::os::fd::OwnedFd;

#[cfg(feature = "tokio")]
use tokio::io::unix::AsyncFd;
#[cfg(feature = "tokio")]
use tokio::net::UdpSocket;

/// Linux runtime backend implemented with tokio.
///
/// Socket receive operations await readiness and return data, an error, or cancellation. Unlike the deterministic mock
/// they never report [`std::io::ErrorKind::WouldBlock`].
#[cfg(feature = "tokio")]
pub struct Tokio {
    link: control::Handle,
    address: control::Handle,
    route: control::Handle,
    events: event::Events,
    ids: socket::Ids,
    packet: socket::Table<AsyncFd<OwnedFd>>,
    icmpv6: socket::Table<AsyncFd<OwnedFd>>,
    udp: socket::Table<UdpSocket>,
    clock: clock::Clock,
    entropy: entropy::Entropy,
}

#[cfg(feature = "tokio")]
impl Tokio {
    /// Opens a route netlink connection and creates the runtime.
    ///
    /// # Panics
    ///
    /// Panics when called outside a tokio runtime context.
    ///
    /// # Errors
    ///
    /// Returns an error when the netlink socket cannot be opened.
    pub fn new() -> std::io::Result<Self> {
        let (link, messages) = control::connect()?;
        let events = event::Events::new(messages, link.clone());

        Ok(Self {
            link: link.clone(),
            address: link.clone(),
            route: link,
            events,
            ids: socket::Ids::new(),
            packet: socket::Table::new(),
            icmpv6: socket::Table::new(),
            udp: socket::Table::new(),
            clock: clock::Clock::new(),
            entropy: entropy::Entropy::new(),
        })
    }
}

#[cfg(feature = "tokio")]
impl net_runtime::Runtime for Tokio {
    type Address = Self;
    type Clock = Self;
    type Entropy = Self;
    type Events = Self;
    type Icmpv6 = Self;
    type Link = Self;
    type Packet = Self;
    type Route = Self;
    type Udp = Self;

    fn address(&self) -> &Self::Address {
        self
    }

    fn clock(&self) -> &Self::Clock {
        self
    }

    fn entropy(&self) -> &Self::Entropy {
        self
    }

    fn events(&self) -> &Self::Events {
        self
    }

    fn icmpv6(&self) -> &Self::Icmpv6 {
        self
    }

    fn link(&self) -> &Self::Link {
        self
    }

    fn packet(&self) -> &Self::Packet {
        self
    }

    fn route(&self) -> &Self::Route {
        self
    }

    fn udp(&self) -> &Self::Udp {
        self
    }
}
