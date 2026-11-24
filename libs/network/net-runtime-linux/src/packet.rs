//! Raw layer 2 packet port over `AF_PACKET` and tokio readiness.

use std::io;
use std::os::fd::AsFd as _;

use ip_method::{SocketKind, SocketSpec};
use net_runtime::SocketId;
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;

use crate::socket;

const BROADCAST: [u8; 6] = [0xff; 6];

impl net_runtime::Packet for super::Tokio {
    fn close(&self, id: SocketId) -> impl Future<Output = io::Result<()>> + Send {
        close(self, id)
    }

    fn open(&self, spec: &SocketSpec) -> impl Future<Output = io::Result<SocketId>> + Send {
        open(self, spec)
    }

    fn recv(&self, id: SocketId, buf: &mut [u8]) -> impl Future<Output = io::Result<usize>> + Send {
        recv(self, id, buf)
    }

    fn send(&self, id: SocketId, frame: &[u8]) -> impl Future<Output = io::Result<usize>> + Send {
        send(self, id, frame)
    }
}

async fn close(runtime: &super::Tokio, id: SocketId) -> io::Result<()> {
    runtime.packet.close(id).await
}

async fn open(runtime: &super::Tokio, spec: &SocketSpec) -> io::Result<SocketId> {
    if spec.kind != SocketKind::RawL2 {
        return Err(socket::invalid_input(
            "packet port requires a raw L2 socket",
        ));
    }

    let fd = packet_v4_linux::open(&spec.link)?;
    // SAFETY: the runtime owns the descriptor for the whole registration.
    let socket = unsafe { AsyncFd::register(fd) }?;

    Ok(runtime.packet.insert(socket).await)
}

async fn recv(runtime: &super::Tokio, id: SocketId, buf: &mut [u8]) -> io::Result<usize> {
    let sockets = runtime.packet.read().await;
    let socket = sockets.get(&id).ok_or_else(socket::not_found)?;

    socket
        .async_io(Interest::READABLE, |fd| {
            packet_v4_linux::recv(fd.as_fd(), buf)
        })
        .await
}

async fn send(runtime: &super::Tokio, id: SocketId, frame: &[u8]) -> io::Result<usize> {
    let sockets = runtime.packet.read().await;
    let socket = sockets.get(&id).ok_or_else(socket::not_found)?;

    socket
        .async_io(Interest::WRITABLE, |fd| {
            packet_v4_linux::send(fd.as_fd(), frame, BROADCAST)
        })
        .await
}
