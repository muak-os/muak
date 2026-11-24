//! Packet and `ICMPv6` port implementations for the deterministic mock runtime.

use alloc::collections::VecDeque;
use std::io::Result;

use super::{Frame, Mock, Socket, State};

impl crate::Packet for Mock {
    fn close(&self, id: crate::SocketId) -> impl Future<Output = Result<()>> + Send {
        std::future::ready(self.write(|state| close(state, id)))
    }

    fn open(
        &self,
        spec: &ip_method::SocketSpec,
    ) -> impl Future<Output = Result<crate::SocketId>> + Send {
        std::future::ready(self.write(|state| open(state, spec, ip_method::SocketKind::RawL2)))
    }

    fn recv(
        &self,
        id: crate::SocketId,
        buf: &mut [u8],
    ) -> impl Future<Output = Result<usize>> + Send {
        std::future::ready(self.write(|state| receive(state, id, buf)))
    }

    fn send(
        &self,
        id: crate::SocketId,
        frame: &[u8],
    ) -> impl Future<Output = Result<usize>> + Send {
        std::future::ready(self.write(|state| send(state, id, frame, None)))
    }
}

impl crate::Icmpv6 for Mock {
    fn close(&self, id: crate::SocketId) -> impl Future<Output = Result<()>> + Send {
        std::future::ready(self.write(|state| close(state, id)))
    }

    fn open(
        &self,
        spec: &ip_method::SocketSpec,
    ) -> impl Future<Output = Result<crate::SocketId>> + Send {
        std::future::ready(self.write(|state| open(state, spec, ip_method::SocketKind::RawIcmpv6)))
    }

    fn recv(
        &self,
        id: crate::SocketId,
        buf: &mut [u8],
    ) -> impl Future<Output = Result<(usize, Option<core::net::Ipv6Addr>)>> + Send {
        std::future::ready(self.write(|state| {
            let frame = take(state, id)?;
            Ok((super::copy_into(&frame.bytes, buf), frame.peer))
        }))
    }

    fn send(
        &self,
        id: crate::SocketId,
        dst: core::net::Ipv6Addr,
        payload: &[u8],
    ) -> impl Future<Output = Result<usize>> + Send {
        std::future::ready(self.write(|state| send(state, id, payload, Some(dst))))
    }
}

fn open(
    state: &mut State,
    spec: &ip_method::SocketSpec,
    expected: ip_method::SocketKind,
) -> Result<crate::SocketId> {
    if spec.kind != expected {
        return Err(super::invalid_input());
    }
    let id = state.next_socket;
    state.next_socket = state.next_socket.wrapping_add(1);
    state.sockets.push(Socket {
        id,
        sent: Vec::new(),
        received: VecDeque::new(),
    });

    Ok(crate::SocketId::new(id))
}

fn close(state: &mut State, id: crate::SocketId) -> Result<()> {
    super::retain_socket(&mut state.sockets, id, |socket| socket.id)
}

fn send(
    state: &mut State,
    id: crate::SocketId,
    bytes: &[u8],
    peer: Option<core::net::Ipv6Addr>,
) -> Result<usize> {
    let socket = state.socket_mut(id)?;
    socket.sent.push(Frame {
        bytes: bytes.to_vec(),
        peer,
    });

    Ok(bytes.len())
}

fn receive(state: &mut State, id: crate::SocketId, buf: &mut [u8]) -> Result<usize> {
    let frame = take(state, id)?;

    Ok(super::copy_into(&frame.bytes, buf))
}

fn take(state: &mut State, id: crate::SocketId) -> Result<Frame> {
    state
        .socket_mut(id)?
        .received
        .pop_front()
        .ok_or_else(super::would_block)
}
