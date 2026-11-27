//! Raw layer 2 packet port over `AF_PACKET` and tokio readiness.

use std::io;
use std::os::fd::AsFd as _;

use ip_method::{SocketKind, SocketSpec};
use net_runtime::SocketId;
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;

use crate::error::Result;
use crate::socket;

const BROADCAST: [u8; 6] = [0xff; 6];

impl net_runtime::Packet for super::Tokio {
    async fn close(&self, id: SocketId) -> net_runtime::error::Result<()> {
        Ok(close(self, id).await?)
    }

    async fn open(&self, spec: &SocketSpec) -> net_runtime::error::Result<SocketId> {
        Ok(open(self, spec).await?)
    }

    async fn recv(&self, id: SocketId, buf: &mut [u8]) -> net_runtime::error::Result<usize> {
        Ok(recv(self, id, buf).await?)
    }

    async fn send(&self, id: SocketId, frame: &[u8]) -> net_runtime::error::Result<usize> {
        Ok(send(self, id, frame).await?)
    }
}

async fn close(runtime: &super::Tokio, id: SocketId) -> Result<()> {
    runtime.packet.close(id).await
}

async fn open(runtime: &super::Tokio, spec: &SocketSpec) -> Result<SocketId> {
    if spec.kind != SocketKind::RawL2 {
        return Err(socket::invalid_input(
            "packet port requires a raw L2 socket",
        ));
    }

    let fd = afpacket_linux::open(&spec.link)?;
    // SAFETY: the runtime owns the descriptor for the whole registration.
    let socket = unsafe { AsyncFd::register(fd) }.map_err(io::Error::from)?;

    Ok(runtime.packet.insert(&runtime.ids, socket).await)
}

async fn recv(runtime: &super::Tokio, id: SocketId, buf: &mut [u8]) -> Result<usize> {
    let sockets = runtime.packet.read().await;
    let socket = sockets.get(&id).ok_or_else(socket::not_found)?;

    Ok(socket
        .async_io(Interest::READABLE, |fd| {
            afpacket_linux::recv(fd.as_fd(), buf).map_err(io::Error::from)
        })
        .await?)
}

async fn send(runtime: &super::Tokio, id: SocketId, frame: &[u8]) -> Result<usize> {
    let sockets = runtime.packet.read().await;
    let socket = sockets.get(&id).ok_or_else(socket::not_found)?;

    Ok(socket
        .async_io(Interest::WRITABLE, |fd| {
            afpacket_linux::send(fd.as_fd(), frame, BROADCAST).map_err(io::Error::from)
        })
        .await?)
}
