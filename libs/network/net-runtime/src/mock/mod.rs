//! Deterministic in-memory runtime for tests.
//!
//! Every piece of state lives in a single mutex so tests can drive time,
//! entropy, links, sockets, and events without touching the operating system.
//! Port operations finish immediately: each returned future is already ready
//! and never yields. The module is compiled under `cfg(test)` as well as the
//! `mock` feature, so the crate's own tests run without a feature flag.

mod address;
mod clock;
mod event;
mod link;
mod route;
mod socket;
#[cfg(test)]
mod tests;

use alloc::collections::VecDeque;
use std::io::{Error, ErrorKind, Result};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Instant, SystemTime};

/// Deterministic in-memory runtime.
pub struct Mock {
    state: Mutex<State>,
}

// Runtime state shared by every port implementation.
struct State {
    now: Instant,
    wall: SystemTime,
    entropy: u64,
    next_link_index: u32,
    next_socket: u32,
    links: Vec<netmodel::Link>,
    bridges: Vec<(netmodel::Name, bool)>,
    addresses: Vec<(netmodel::Name, netmodel::Address)>,
    default_routes: Vec<(netmodel::Name, core::net::IpAddr)>,
    events: VecDeque<netmodel::LinkEvent>,
    sockets: Vec<Socket>,
}

// One open socket and its queued traffic.
struct Socket {
    id: u32,
    sent: Vec<Frame>,
    received: VecDeque<Frame>,
}

// A stored frame with the peer address it was sent to or received from.
struct Frame {
    bytes: Vec<u8>,
    peer: Option<core::net::Ipv6Addr>,
}

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
    /// Returns [`ErrorKind::NotFound`] when the socket is closed.
    pub fn push_inbound(&self, id: crate::SocketId, bytes: &[u8]) -> Result<()> {
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
    /// Returns [`ErrorKind::NotFound`] when the socket is closed.
    pub fn push_inbound_from(
        &self,
        id: crate::SocketId,
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

    /// Returns the frames sent on a socket, in send order.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::NotFound`] when the socket is closed.
    pub fn sent(&self, id: crate::SocketId) -> Result<Vec<Vec<u8>>> {
        self.read(|state| sent_frames(state, id))
    }

    /// Returns the frames sent on a socket with their peer address, in send order.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::NotFound`] when the socket is closed.
    pub fn sent_with_peer(
        &self,
        id: crate::SocketId,
    ) -> Result<Vec<(Vec<u8>, Option<core::net::Ipv6Addr>)>> {
        self.read(|state| sent_frames_with_peer(state, id))
    }

    /// Returns the default routes currently installed.
    #[must_use = "callers must inspect the installed routes"]
    pub fn default_routes(&self) -> Vec<(netmodel::Name, core::net::IpAddr)> {
        self.read(|state| state.default_routes.clone())
    }

    fn push_frame(&self, id: crate::SocketId, frame: Frame) -> Result<()> {
        self.write(|state| queue_inbound(state, id, frame))
    }

    fn read<T>(&self, project: impl FnOnce(&State) -> T) -> T {
        project(&self.lock())
    }

    fn write<T>(&self, project: impl FnOnce(&mut State) -> T) -> T {
        project(&mut self.lock())
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl State {
    fn socket(&self, id: crate::SocketId) -> Result<&Socket> {
        self.sockets
            .iter()
            .find(|socket| socket.id == id.get())
            .ok_or_else(not_found)
    }

    fn socket_mut(&mut self, id: crate::SocketId) -> Result<&mut Socket> {
        self.sockets
            .iter_mut()
            .find(|socket| socket.id == id.get())
            .ok_or_else(not_found)
    }
}

impl crate::Runtime for Mock {
    type Address = Self;
    type Clock = Self;
    type Entropy = Self;
    type Events = Self;
    type Icmpv6 = Self;
    type Link = Self;
    type Packet = Self;
    type Route = Self;
}

fn sent_frames(state: &State, id: crate::SocketId) -> Result<Vec<Vec<u8>>> {
    let socket = state.socket(id)?;
    Ok(socket
        .sent
        .iter()
        .map(|frame| frame.bytes.clone())
        .collect())
}

fn sent_frames_with_peer(
    state: &State,
    id: crate::SocketId,
) -> Result<Vec<(Vec<u8>, Option<core::net::Ipv6Addr>)>> {
    let socket = state.socket(id)?;
    Ok(socket
        .sent
        .iter()
        .map(|frame| (frame.bytes.clone(), frame.peer))
        .collect())
}

fn require_link(state: &State, name: &netmodel::Name) -> Result<()> {
    if state.links.iter().any(|link| &link.name == name) {
        return Ok(());
    }
    Err(not_found())
}

fn queue_inbound(state: &mut State, id: crate::SocketId, frame: Frame) -> Result<()> {
    state
        .socket_mut(id)
        .map(|socket| socket.received.push_back(frame))
}

fn not_found() -> Error {
    Error::from(ErrorKind::NotFound)
}

fn would_block() -> Error {
    Error::from(ErrorKind::WouldBlock)
}
