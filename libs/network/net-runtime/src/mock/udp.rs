//! `UDP` port implementation for the deterministic mock runtime.

use alloc::collections::VecDeque;
use core::net::SocketAddr;
use std::io::Result;

use super::{Mock, State};
use crate::{SocketId, Udp};

pub(super) struct Datagram {
    pub(super) id: u32,
    pub(super) bind: Option<SocketAddr>,
    pub(super) peer: Option<SocketAddr>,
    pub(super) sent: Vec<Vec<u8>>,
    pub(super) received: VecDeque<Vec<u8>>,
}

impl Udp for Mock {
    fn close(&self, id: SocketId) -> impl Future<Output = Result<()>> + Send {
        std::future::ready(self.write(|state| close(state, id)))
    }

    fn open(&self, spec: &ip_method::SocketSpec) -> impl Future<Output = Result<SocketId>> + Send {
        std::future::ready(self.write(|state| open(state, spec)))
    }

    fn recv(&self, id: SocketId, buf: &mut [u8]) -> impl Future<Output = Result<usize>> + Send {
        std::future::ready(self.write(|state| receive(state, id, buf)))
    }

    fn send(&self, id: SocketId, payload: &[u8]) -> impl Future<Output = Result<usize>> + Send {
        std::future::ready(self.write(|state| send(state, id, payload)))
    }
}

fn open(state: &mut State, spec: &ip_method::SocketSpec) -> Result<SocketId> {
    if spec.kind != ip_method::SocketKind::Udp {
        return Err(super::invalid_input());
    }
    let id = state.next_socket;
    state.next_socket = state.next_socket.wrapping_add(1);
    state.udp.push(Datagram {
        id,
        bind: spec.bind,
        peer: spec.peer,
        sent: Vec::new(),
        received: VecDeque::new(),
    });

    Ok(SocketId::new(id))
}

fn close(state: &mut State, id: SocketId) -> Result<()> {
    super::retain_socket(&mut state.udp, id, |datagram| datagram.id)
}

fn send(state: &mut State, id: SocketId, payload: &[u8]) -> Result<usize> {
    let datagram = state.udp_mut(id)?;
    datagram.sent.push(payload.to_vec());

    Ok(payload.len())
}

fn receive(state: &mut State, id: SocketId, buf: &mut [u8]) -> Result<usize> {
    let payload = state
        .udp_mut(id)?
        .received
        .pop_front()
        .ok_or_else(super::would_block)?;

    Ok(super::copy_into(&payload, buf))
}
