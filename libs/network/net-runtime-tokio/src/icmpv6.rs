//! Raw `ICMPv6` port over a socket and tokio readiness.

use core::net::Ipv6Addr;
use std::io;
use std::os::fd::AsFd as _;

use ip_method::{SocketKind, SocketSpec};
use net_runtime::SocketId;
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;

use crate::error::Result;
use crate::socket;

impl net_runtime::Icmpv6 for super::Tokio {
    async fn close(&self, id: SocketId) -> net_runtime::error::Result<()> {
        Ok(close(self, id).await?)
    }

    async fn open(&self, spec: &SocketSpec) -> net_runtime::error::Result<SocketId> {
        Ok(open(self, spec).await?)
    }

    async fn recv(
        &self,
        id: SocketId,
        buf: &mut [u8],
    ) -> net_runtime::error::Result<(usize, Option<Ipv6Addr>)> {
        Ok(recv(self, id, buf).await?)
    }

    async fn send(
        &self,
        id: SocketId,
        dst: Ipv6Addr,
        payload: &[u8],
    ) -> net_runtime::error::Result<usize> {
        Ok(send(self, id, dst, payload).await?)
    }
}

async fn close(runtime: &super::Tokio, id: SocketId) -> Result<()> {
    runtime.icmpv6.close(id).await
}

async fn open(runtime: &super::Tokio, spec: &SocketSpec) -> Result<SocketId> {
    if spec.kind != SocketKind::RawIcmpv6 {
        return Err(socket::invalid_input(
            "icmpv6 port requires a raw ICMPv6 socket",
        ));
    }

    let fd = icmpv6_linux::open(&spec.link)?;
    // SAFETY: the runtime owns the descriptor for the whole registration.
    let socket = unsafe { AsyncFd::register(fd) }.map_err(io::Error::from)?;

    Ok(runtime.icmpv6.insert(&runtime.ids, socket).await)
}

async fn recv(
    runtime: &super::Tokio,
    id: SocketId,
    buf: &mut [u8],
) -> Result<(usize, Option<Ipv6Addr>)> {
    let sockets = runtime.icmpv6.read().await;
    let socket = sockets.get(&id).ok_or_else(socket::not_found)?;

    Ok(socket
        .async_io(Interest::READABLE, |fd| {
            icmpv6_linux::recv(fd.as_fd(), buf).map_err(io::Error::from)
        })
        .await?)
}

async fn send(
    runtime: &super::Tokio,
    id: SocketId,
    dst: Ipv6Addr,
    payload: &[u8],
) -> Result<usize> {
    let sockets = runtime.icmpv6.read().await;
    let socket = sockets.get(&id).ok_or_else(socket::not_found)?;

    Ok(socket
        .async_io(Interest::WRITABLE, |fd| {
            icmpv6_linux::send(fd.as_fd(), dst, payload).map_err(io::Error::from)
        })
        .await?)
}
