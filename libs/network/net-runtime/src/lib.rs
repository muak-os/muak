//! Platform-agnostic network runtime ports plus a deterministic in-memory mock behind the `mock` feature.

#![warn(missing_docs)]

pub mod error;

#[cfg(any(test, feature = "mock"))]
extern crate alloc;

#[cfg(any(test, feature = "mock"))]
pub mod mock;

use crate::error::Result;

/// Identifier of a socket opened by the runtime, unique across all of its ports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SocketId(u32);

impl SocketId {
    /// Creates a socket identifier from a runtime handle value.
    #[must_use = "the identifier must be stored or passed back to the runtime"]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the runtime handle value of the socket.
    #[must_use = "callers need the handle value to address the socket"]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Link control capability.
pub trait Link {
    /// Detaches a port link from its master.
    ///
    /// # Errors
    ///
    /// Returns an error when the link does not exist or the change fails.
    fn detach(&self, port: &netmodel::Name) -> impl Future<Output = Result<()>> + Send;

    /// Ensures a bridge device exists with the requested spanning tree setting.
    ///
    /// # Errors
    ///
    /// Returns an error when the bridge cannot be created or updated.
    fn ensure_bridge(
        &self,
        name: &netmodel::Name,
        stp: bool,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Looks up a single link by interface name.
    ///
    /// # Errors
    ///
    /// Returns an error when the link query fails.
    fn link(
        &self,
        name: &netmodel::Name,
    ) -> impl Future<Output = Result<Option<netmodel::Link>>> + Send;

    /// Lists every network link known to the kernel.
    ///
    /// # Errors
    ///
    /// Returns an error when the link query fails.
    fn links(&self) -> impl Future<Output = Result<Vec<netmodel::Link>>> + Send;

    /// Administratively brings a link down.
    ///
    /// # Errors
    ///
    /// Returns an error when the link does not exist or the change fails.
    fn set_down(&self, name: &netmodel::Name) -> impl Future<Output = Result<()>> + Send;

    /// Enslaves a port link to a master link.
    ///
    /// # Errors
    ///
    /// Returns an error when either link does not exist or the change fails.
    fn set_master(
        &self,
        port: &netmodel::Name,
        master: &netmodel::Name,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Administratively brings a link up.
    ///
    /// # Errors
    ///
    /// Returns an error when the link does not exist or the change fails.
    fn set_up(&self, name: &netmodel::Name) -> impl Future<Output = Result<()>> + Send;
}

/// Address management capability.
pub trait Address {
    /// Adds an address to a link.
    ///
    /// # Errors
    ///
    /// Returns an error when the address cannot be added.
    fn add(
        &self,
        link: &netmodel::Name,
        address: netmodel::Address,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Lists the addresses assigned to a link.
    ///
    /// # Errors
    ///
    /// Returns an error when the address query fails.
    fn list(
        &self,
        link: &netmodel::Name,
    ) -> impl Future<Output = Result<Vec<netmodel::Address>>> + Send;

    /// Removes an address from a link.
    ///
    /// # Errors
    ///
    /// Returns an error when the address cannot be removed.
    fn remove(
        &self,
        link: &netmodel::Name,
        address: netmodel::Address,
    ) -> impl Future<Output = Result<()>> + Send;
}

/// Default route management capability.
pub trait Route {
    /// Ensures a default route through the given gateway on the link.
    ///
    /// # Errors
    ///
    /// Returns an error when the route cannot be installed.
    fn ensure_default(
        &self,
        link: &netmodel::Name,
        gateway: core::net::IpAddr,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Removes the default route through the given gateway on the link.
    ///
    /// # Errors
    ///
    /// Returns an error when the route cannot be removed.
    fn remove_default(
        &self,
        link: &netmodel::Name,
        gateway: core::net::IpAddr,
    ) -> impl Future<Output = Result<()>> + Send;
}

/// Link event delivery capability.
pub trait Events {
    /// Waits for the next link event.
    ///
    /// # Errors
    ///
    /// Returns an error when link monitoring fails or the stream closes. [`std::io::ErrorKind::WouldBlock`] means no
    /// event is ready yet.
    fn next(&self) -> impl Future<Output = Result<netmodel::LinkEvent>> + Send;
}

/// Raw packet socket capability.
pub trait Packet {
    /// Closes an open socket.
    ///
    /// # Errors
    ///
    /// Returns an error when the socket is already closed.
    fn close(&self, id: SocketId) -> impl Future<Output = Result<()>> + Send;

    /// Opens a packet socket described by the specification.
    ///
    /// # Errors
    ///
    /// Returns an error when the socket cannot be opened.
    fn open(&self, spec: &ip_method::SocketSpec) -> impl Future<Output = Result<SocketId>> + Send;

    /// Receives a frame from an open socket into the buffer.
    ///
    /// # Errors
    ///
    /// Returns an error when the socket is closed or the receive fails. [`std::io::ErrorKind::WouldBlock`] means no
    /// frame is ready yet.
    fn recv(&self, id: SocketId, buf: &mut [u8]) -> impl Future<Output = Result<usize>> + Send;

    /// Sends a frame on an open socket.
    ///
    /// # Errors
    ///
    /// Returns an error when the socket is closed or the send fails.
    fn send(&self, id: SocketId, frame: &[u8]) -> impl Future<Output = Result<usize>> + Send;
}

/// Datagram (`UDP`) socket capability.
///
/// Every socket kind shares [`ip_method::SocketSpec`], so `bind` and `peer` are not `UDP`-specific. Implementations
/// should retain those endpoints for the lifetime of the socket opened by [`open`](Udp::open). Unlike the raw
/// [`Packet`] and [`Icmpv6`] ports, a `UDP` socket is addressed by its endpoints rather than by a frame destination.
pub trait Udp {
    /// Closes an open socket.
    ///
    /// # Errors
    ///
    /// Returns an error when the socket is already closed.
    fn close(&self, id: SocketId) -> impl Future<Output = Result<()>> + Send;

    /// Opens a `UDP` socket described by the specification.
    ///
    /// # Errors
    ///
    /// Returns an error when the socket cannot be opened.
    fn open(&self, spec: &ip_method::SocketSpec) -> impl Future<Output = Result<SocketId>> + Send;

    /// Receives a datagram payload from an open socket into the buffer.
    ///
    /// # Errors
    ///
    /// Returns an error when the socket is closed or the receive fails. [`std::io::ErrorKind::WouldBlock`] means no
    /// datagram is ready yet.
    fn recv(&self, id: SocketId, buf: &mut [u8]) -> impl Future<Output = Result<usize>> + Send;

    /// Sends a datagram payload on an open socket.
    ///
    /// # Errors
    ///
    /// Returns an error when the socket is closed or the send fails.
    fn send(&self, id: SocketId, payload: &[u8]) -> impl Future<Output = Result<usize>> + Send;
}

/// Raw `ICMPv6` socket capability.
pub trait Icmpv6 {
    /// Closes an open socket.
    ///
    /// # Errors
    ///
    /// Returns an error when the socket is already closed.
    fn close(&self, id: SocketId) -> impl Future<Output = Result<()>> + Send;

    /// Opens an `ICMPv6` socket described by the specification.
    ///
    /// # Errors
    ///
    /// Returns an error when the socket cannot be opened.
    fn open(&self, spec: &ip_method::SocketSpec) -> impl Future<Output = Result<SocketId>> + Send;

    /// Receives an `ICMPv6` payload and its source address.
    ///
    /// # Errors
    ///
    /// Returns an error when the socket is closed or the receive fails. [`std::io::ErrorKind::WouldBlock`] means no
    /// frame is ready yet.
    fn recv(
        &self,
        id: SocketId,
        buf: &mut [u8],
    ) -> impl Future<Output = Result<(usize, Option<core::net::Ipv6Addr>)>> + Send;

    /// Sends an `ICMPv6` payload to the destination address.
    ///
    /// # Errors
    ///
    /// Returns an error when the socket is closed or the send fails.
    fn send(
        &self,
        id: SocketId,
        dst: core::net::Ipv6Addr,
        payload: &[u8],
    ) -> impl Future<Output = Result<usize>> + Send;
}

/// Monotonic and wall clock capability.
pub trait Clock {
    /// Returns the monotonic instant used for scheduling.
    fn now(&self) -> std::time::Instant;

    /// Returns the current wall clock time.
    fn wall_now(&self) -> std::time::SystemTime;
}

/// Entropy capability.
pub trait Entropy {
    /// Fills the buffer with random bytes.
    fn fill(&self, bytes: &mut [u8]);
}

/// Bundle of the ports a network runtime provides.
pub trait Runtime {
    /// Address management port.
    type Address: Address;
    /// Clock port.
    type Clock: Clock;
    /// Entropy port.
    type Entropy: Entropy;
    /// Link event port.
    type Events: Events;
    /// `ICMPv6` socket port.
    type Icmpv6: Icmpv6;
    /// Link control port.
    type Link: Link;
    /// Packet socket port.
    type Packet: Packet;
    /// Default route management port.
    type Route: Route;
    /// Datagram socket port.
    type Udp: Udp;

    /// Returns the address management port.
    fn address(&self) -> &Self::Address;

    /// Returns the clock port.
    fn clock(&self) -> &Self::Clock;

    /// Returns the entropy port.
    fn entropy(&self) -> &Self::Entropy;

    /// Returns the link event port.
    fn events(&self) -> &Self::Events;

    /// Returns the `ICMPv6` socket port.
    fn icmpv6(&self) -> &Self::Icmpv6;

    /// Returns the link control port.
    fn link(&self) -> &Self::Link;

    /// Returns the packet socket port.
    fn packet(&self) -> &Self::Packet;

    /// Returns the default route management port.
    fn route(&self) -> &Self::Route;

    /// Returns the datagram socket port.
    fn udp(&self) -> &Self::Udp;
}
