//! Deterministic in-memory runtime for driving time, entropy, links, sockets, and events in tests.

mod address;
mod clock;
mod event;
mod handle;
mod link;
mod route;
mod socket;
mod udp;

use alloc::collections::VecDeque;
use std::io::{Error, ErrorKind};
use std::sync::Mutex;
use std::time::{Instant, SystemTime};

use crate::error::{Result, RuntimeError};
use crate::{Runtime, SocketId};

/// Deterministic in-memory runtime.
pub struct Mock {
    state: Mutex<State>,
}

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
    udp: Vec<udp::Datagram>,
}

struct Socket {
    id: u32,
    sent: Vec<Frame>,
    received: VecDeque<Frame>,
}

struct Frame {
    bytes: Vec<u8>,
    peer: Option<core::net::Ipv6Addr>,
}

impl State {
    fn socket(&self, id: SocketId) -> Result<&Socket> {
        self.sockets
            .iter()
            .find(|socket| socket.id == id.get())
            .ok_or_else(not_found)
    }

    fn socket_mut(&mut self, id: SocketId) -> Result<&mut Socket> {
        self.sockets
            .iter_mut()
            .find(|socket| socket.id == id.get())
            .ok_or_else(not_found)
    }

    fn udp(&self, id: SocketId) -> Result<&udp::Datagram> {
        self.udp
            .iter()
            .find(|datagram| datagram.id == id.get())
            .ok_or_else(not_found)
    }

    fn udp_mut(&mut self, id: SocketId) -> Result<&mut udp::Datagram> {
        self.udp
            .iter_mut()
            .find(|datagram| datagram.id == id.get())
            .ok_or_else(not_found)
    }
}

impl Runtime for Mock {
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

fn sent_frames(state: &State, id: SocketId) -> Result<Vec<Vec<u8>>> {
    let socket = state.socket(id)?;

    Ok(socket
        .sent
        .iter()
        .map(|frame| frame.bytes.clone())
        .collect())
}

fn sent_frames_with_peer(
    state: &State,
    id: SocketId,
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

fn queue_inbound(state: &mut State, id: SocketId, frame: Frame) -> Result<()> {
    state
        .socket_mut(id)
        .map(|socket| socket.received.push_back(frame))
}

fn retain_socket<T>(table: &mut Vec<T>, id: SocketId, id_of: impl Fn(&T) -> u32) -> Result<()> {
    let before = table.len();
    table.retain(|entry| id_of(entry) != id.get());
    if table.len() == before {
        return Err(not_found());
    }

    Ok(())
}

fn copy_into(bytes: &[u8], buf: &mut [u8]) -> usize {
    let len = bytes.len().min(buf.len());
    if let (Some(source), Some(target)) = (bytes.get(..len), buf.get_mut(..len)) {
        target.copy_from_slice(source);
    }

    len
}

fn not_found() -> RuntimeError {
    RuntimeError::Io(Error::from(ErrorKind::NotFound))
}

fn would_block() -> RuntimeError {
    RuntimeError::Io(Error::from(ErrorKind::WouldBlock))
}

fn invalid_input() -> RuntimeError {
    RuntimeError::Io(Error::from(ErrorKind::InvalidInput))
}
