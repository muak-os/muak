//! Socket creation for DHCP exchanges.

use core::future::Future;
use core::net::Ipv4Addr;

use dhcp_proto::DHCP_CLIENT_PORT;
use packet::raw::Socket;
use packet::socket::bind_device;
use tokio::net::UdpSocket;

use crate::{Failure, Result};

/// Creates the sockets an exchange needs.
pub trait Connector: Clone + Send + Sync + 'static {
    /// Opens a raw `AF_PACKET` socket bound to `interface` for broadcast exchanges.
    fn create_raw(&self, interface: &str) -> impl Future<Output = Result<Socket>> + Send;

    /// Opens a unicast UDP socket bound to `interface` with `src_ip`.
    fn create_unicast(
        &self,
        interface: &str,
        src_ip: Ipv4Addr,
    ) -> impl Future<Output = Result<UdpSocket>> + Send;
}

/// Connector backed by the kernel.
#[derive(Clone, Copy, Debug, Default)]
pub struct System;

impl Connector for System {
    fn create_raw(&self, interface: &str) -> impl Future<Output = Result<Socket>> + Send {
        std::future::ready(Socket::open(interface).map_err(Failure::from))
    }

    async fn create_unicast(&self, interface: &str, src_ip: Ipv4Addr) -> Result<UdpSocket> {
        let socket = UdpSocket::bind((src_ip, DHCP_CLIENT_PORT)).await?;
        bind_device(&socket, interface)?;

        Ok(socket)
    }
}
