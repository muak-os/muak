//! Test-facing handle methods for the deterministic mock runtime.

use alloc::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Instant, SystemTime};

use super::{Frame, Mock, State};
use crate::SocketId;
use crate::error::Result;

impl Mock {
    /// Creates a runtime whose clocks start at `now` and `wall`.
    #[must_use = "the runtime owns the mock state under test"]
    pub const fn new(now: Instant, wall: SystemTime) -> Self {
        Self {
            state: Mutex::new(State {
                now,
                wall,
                entropy: 0,
                next_link_index: 1,
                next_socket: 0,
                links: Vec::new(),
                bridges: Vec::new(),
                addresses: Vec::new(),
                default_routes: Vec::new(),
                events: VecDeque::new(),
                sockets: Vec::new(),
                udp: Vec::new(),
            }),
        }
    }

    /// Sets the monotonic clock.
    pub fn set_now(&self, now: Instant) {
        self.write(|state| state.now = now);
    }

    /// Sets the wall clock.
    pub fn set_wall(&self, wall: SystemTime) {
        self.write(|state| state.wall = wall);
    }

    /// Adds a link or replaces the link with the same name.
    pub fn set_link(&self, link: netmodel::Link) {
        self.write(
            |state| match state.links.iter_mut().find(|known| known.name == link.name) {
                Some(known) => *known = link,
                None => state.links.push(link),
            },
        );
    }

    /// Returns the spanning tree setting recorded for a bridge, if known.
    #[must_use = "callers must inspect the recorded bridge setting"]
    pub fn bridge_stp(&self, name: &netmodel::Name) -> Option<bool> {
        self.read(|state| {
            state
                .bridges
                .iter()
                .find(|entry| &entry.0 == name)
                .map(|entry| entry.1)
        })
    }

    /// Appends a link event to the queue read by [`crate::Events`].
    pub fn push_event(&self, event: netmodel::LinkEvent) {
        self.write(|state| state.events.push_back(event));
    }

    /// Queues a frame for a socket opened through [`crate::Packet`].
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::RuntimeError::Io`] carrying [`std::io::ErrorKind::NotFound`] when the socket is closed.
    pub fn push_inbound(&self, id: SocketId, bytes: &[u8]) -> Result<()> {
        self.push_frame(
            id,
            Frame {
                bytes: bytes.to_vec(),
                peer: None,
            },
        )
    }

    /// Queues an `ICMPv6` frame with its source for a socket opened through [`crate::Icmpv6`].
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::RuntimeError::Io`] carrying [`std::io::ErrorKind::NotFound`] when the socket is closed.
    pub fn push_inbound_from(
        &self,
        id: SocketId,
        bytes: &[u8],
        source: core::net::Ipv6Addr,
    ) -> Result<()> {
        self.push_frame(
            id,
            Frame {
                bytes: bytes.to_vec(),
                peer: Some(source),
            },
        )
    }

    /// Queues a datagram for a socket opened through [`crate::Udp`].
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::RuntimeError::Io`] carrying [`std::io::ErrorKind::NotFound`] when the socket is closed.
    pub fn push_inbound_udp(&self, id: SocketId, bytes: &[u8]) -> Result<()> {
        self.write(|state| {
            state
                .udp_mut(id)
                .map(|datagram| datagram.received.push_back(bytes.to_vec()))
        })
    }

    /// Returns the payloads sent on a `UDP` socket, in send order.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::RuntimeError::Io`] carrying [`std::io::ErrorKind::NotFound`] when the socket is closed.
    pub fn udp_sent(&self, id: SocketId) -> Result<Vec<Vec<u8>>> {
        self.read(|state| state.udp(id).map(|datagram| datagram.sent.clone()))
    }

    /// Returns the bind and peer endpoints recorded for a `UDP` socket.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::RuntimeError::Io`] carrying [`std::io::ErrorKind::NotFound`] when the socket is closed.
    pub fn udp_endpoints(
        &self,
        id: SocketId,
    ) -> Result<(Option<core::net::SocketAddr>, Option<core::net::SocketAddr>)> {
        self.read(|state| state.udp(id).map(|datagram| (datagram.bind, datagram.peer)))
    }

    /// Returns the frames sent on a socket, in send order.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::RuntimeError::Io`] carrying [`std::io::ErrorKind::NotFound`] when the socket is closed.
    pub fn sent(&self, id: SocketId) -> Result<Vec<Vec<u8>>> {
        self.read(|state| super::sent_frames(state, id))
    }

    /// Returns the frames sent on a socket with their peer address, in send order.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::RuntimeError::Io`] carrying [`std::io::ErrorKind::NotFound`] when the socket is closed.
    pub fn sent_with_peer(
        &self,
        id: SocketId,
    ) -> Result<Vec<(Vec<u8>, Option<core::net::Ipv6Addr>)>> {
        self.read(|state| super::sent_frames_with_peer(state, id))
    }

    /// Returns the default routes currently installed.
    #[must_use = "callers must inspect the installed routes"]
    pub fn default_routes(&self) -> Vec<(netmodel::Name, core::net::IpAddr)> {
        self.read(|state| state.default_routes.clone())
    }

    fn push_frame(&self, id: SocketId, frame: Frame) -> Result<()> {
        self.write(|state| super::queue_inbound(state, id, frame))
    }

    pub(super) fn read<T>(&self, project: impl FnOnce(&State) -> T) -> T {
        project(&self.lock())
    }

    pub(super) fn write<T>(&self, project: impl FnOnce(&mut State) -> T) -> T {
        project(&mut self.lock())
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
