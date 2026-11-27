//! Datagram port over tokio `UDP` sockets.

use std::io;

use ip_method::{SocketKind, SocketSpec};
use net_runtime::SocketId;
use tokio::net::UdpSocket;

use crate::socket;

impl net_runtime::Udp for super::Tokio {
    fn close(&self, id: SocketId) -> impl Future<Output = io::Result<()>> + Send {
        close(self, id)
    }

    fn open(&self, spec: &SocketSpec) -> impl Future<Output = io::Result<SocketId>> + Send {
        open(self, spec)
    }

    fn recv(&self, id: SocketId, buf: &mut [u8]) -> impl Future<Output = io::Result<usize>> + Send {
        recv(self, id, buf)
    }

    fn send(&self, id: SocketId, payload: &[u8]) -> impl Future<Output = io::Result<usize>> + Send {
        send(self, id, payload)
    }
}

async fn close(runtime: &super::Tokio, id: SocketId) -> io::Result<()> {
    runtime.udp.close(id).await
}

async fn open(runtime: &super::Tokio, spec: &SocketSpec) -> io::Result<SocketId> {
    if spec.kind != SocketKind::Udp {
        return Err(socket::invalid_input("udp port requires a datagram socket"));
    }
    let bind = spec
        .bind
        .ok_or_else(|| socket::invalid_input("udp socket requires a local address"))?;

    let socket = UdpSocket::bind(bind).await?;
    socket.bind_device(Some(spec.link.as_str().as_bytes()))?;
    if let Some(peer) = spec.peer {
        socket.connect(peer).await?;
    }

    Ok(runtime.udp.insert(&runtime.ids, socket).await)
}

async fn recv(runtime: &super::Tokio, id: SocketId, buf: &mut [u8]) -> io::Result<usize> {
    let sockets = runtime.udp.read().await;
    let socket = sockets.get(&id).ok_or_else(socket::not_found)?;

    socket.recv(buf).await
}

async fn send(runtime: &super::Tokio, id: SocketId, payload: &[u8]) -> io::Result<usize> {
    let sockets = runtime.udp.read().await;
    let socket = sockets.get(&id).ok_or_else(socket::not_found)?;

    socket.send(payload).await
}
