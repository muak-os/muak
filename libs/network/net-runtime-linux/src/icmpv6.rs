//! Raw `ICMPv6` port over a socket and tokio readiness.

use core::net::Ipv6Addr;
use std::io;
use std::os::fd::AsFd as _;

use ip_method::{SocketKind, SocketSpec};
use net_runtime::SocketId;
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;

use crate::socket;

impl net_runtime::Icmpv6 for super::Tokio {
    fn close(&self, id: SocketId) -> impl Future<Output = io::Result<()>> + Send {
        close(self, id)
    }

    fn open(&self, spec: &SocketSpec) -> impl Future<Output = io::Result<SocketId>> + Send {
        open(self, spec)
    }

    fn recv(
        &self,
        id: SocketId,
        buf: &mut [u8],
    ) -> impl Future<Output = io::Result<(usize, Option<Ipv6Addr>)>> + Send {
        recv(self, id, buf)
    }

    fn send(
        &self,
        id: SocketId,
        dst: Ipv6Addr,
        payload: &[u8],
    ) -> impl Future<Output = io::Result<usize>> + Send {
        send(self, id, dst, payload)
    }
}

async fn close(runtime: &super::Tokio, id: SocketId) -> io::Result<()> {
    runtime.icmpv6.close(id).await
}

async fn open(runtime: &super::Tokio, spec: &SocketSpec) -> io::Result<SocketId> {
    if spec.kind != SocketKind::RawIcmpv6 {
        return Err(socket::invalid_input(
            "icmpv6 port requires a raw ICMPv6 socket",
        ));
    }

    let fd = icmpv6_linux::open(&spec.link)?;
    // SAFETY: the runtime owns the descriptor for the whole registration.
    let socket = unsafe { AsyncFd::register(fd) }?;

    Ok(runtime.icmpv6.insert(socket).await)
}

async fn recv(
    runtime: &super::Tokio,
    id: SocketId,
    buf: &mut [u8],
) -> io::Result<(usize, Option<Ipv6Addr>)> {
    let sockets = runtime.icmpv6.read().await;
    let socket = sockets.get(&id).ok_or_else(socket::not_found)?;

    socket
        .async_io(Interest::READABLE, |fd| icmpv6_linux::recv(fd.as_fd(), buf))
        .await
}

async fn send(
    runtime: &super::Tokio,
    id: SocketId,
    dst: Ipv6Addr,
    payload: &[u8],
) -> io::Result<usize> {
    let sockets = runtime.icmpv6.read().await;
    let socket = sockets.get(&id).ok_or_else(socket::not_found)?;

    socket
        .async_io(Interest::WRITABLE, |fd| {
            icmpv6_linux::send(fd.as_fd(), dst, payload)
        })
        .await
}
