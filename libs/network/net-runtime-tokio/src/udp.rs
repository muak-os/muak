//! Datagram port over tokio `UDP` sockets.

use ip_method::{SocketKind, SocketSpec};
use net_runtime::SocketId;
use tokio::net::UdpSocket;

use crate::error::Result;
use crate::socket;

impl net_runtime::Udp for super::Tokio {
    async fn close(&self, id: SocketId) -> net_runtime::error::Result<()> {
        Ok(close(self, id).await?)
    }

    async fn open(&self, spec: &SocketSpec) -> net_runtime::error::Result<SocketId> {
        Ok(open(self, spec).await?)
    }

    async fn recv(&self, id: SocketId, buf: &mut [u8]) -> net_runtime::error::Result<usize> {
        Ok(recv(self, id, buf).await?)
    }

    async fn send(&self, id: SocketId, payload: &[u8]) -> net_runtime::error::Result<usize> {
        Ok(send(self, id, payload).await?)
    }
}

async fn close(runtime: &super::Tokio, id: SocketId) -> Result<()> {
    runtime.udp.close(id).await
}

async fn open(runtime: &super::Tokio, spec: &SocketSpec) -> Result<SocketId> {
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

async fn recv(runtime: &super::Tokio, id: SocketId, buf: &mut [u8]) -> Result<usize> {
    let sockets = runtime.udp.read().await;
    let socket = sockets.get(&id).ok_or_else(socket::not_found)?;

    Ok(socket.recv(buf).await?)
}

async fn send(runtime: &super::Tokio, id: SocketId, payload: &[u8]) -> Result<usize> {
    let sockets = runtime.udp.read().await;
    let socket = sockets.get(&id).ok_or_else(socket::not_found)?;

    Ok(socket.send(payload).await?)
}
