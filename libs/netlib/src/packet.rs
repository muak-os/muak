//! Async `AF_PACKET`/`SOCK_DGRAM` raw socket bound to a single interface.

use std::io;
use std::os::fd::{AsFd as _, BorrowedFd, OwnedFd};

use rustix::net::addr::{SocketAddrArg, SocketAddrLen, SocketAddrOpaque};
use rustix::net::{
    AddressFamily, RecvFlags, SendFlags, SocketFlags, SocketType, bind, eth, netdevice, recv,
    sendto, socket_with,
};
use thiserror::Error;
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;

use crate::socket::{Failure as SocketFailure, bind_device};

#[derive(Debug, Error)]
pub enum Failure {
    #[error("failed to create AF_PACKET socket: {0}")]
    Create(#[source] io::Error),
    #[error("failed to bind AF_PACKET socket to interface {device}: {source}")]
    BindIface {
        device: String,
        #[source]
        source: io::Error,
    },
    #[error("failed to bind AF_PACKET socket via SO_BINDTODEVICE: {0}")]
    BindDevice(#[from] SocketFailure),
    #[error("failed to look up interface index for {device}: {source}")]
    Index {
        device: String,
        #[source]
        source: io::Error,
    },
    #[error("I/O error on packet socket: {0}")]
    Io(#[from] io::Error),
}

pub type Result<T> = core::result::Result<T, Failure>;

const ETH_ALEN: usize = 6;
const ETH_ALEN_U8: u8 = 6;
const ETH_PROTOCOL_IP: u16 = 0x0800;
const SOCK_ADDR_LL_LEN: SocketAddrLen = 20;
/// Standard link-layer broadcast address.
pub const ETH_BROADCAST: [u8; ETH_ALEN] = [0xff; ETH_ALEN];

/// Stable kernel UAPI `struct sockaddr_ll` from `<linux/if_packet.h>`.
#[repr(C)]
struct SockAddrLl {
    family: u16,
    protocol: u16,
    if_index: i32,
    hatype: u16,
    packet_type: u8,
    hardware_addr_len: u8,
    address: [u8; 8],
}

// SAFETY: `SockAddrLl` is `repr(C)` matching the kernel ABI for `sockaddr_ll`,
// and its size is the exact length the kernel expects for `AF_PACKET` syscalls.
unsafe impl SocketAddrArg for SockAddrLl {
    unsafe fn with_sockaddr<R>(
        &self,
        f: impl FnOnce(*const SocketAddrOpaque, SocketAddrLen) -> R,
    ) -> R {
        f(
            core::ptr::from_ref(self).cast::<SocketAddrOpaque>(),
            SOCK_ADDR_LL_LEN,
        )
    }
}

/// Async DHCP raw socket bound to a specific interface, filtered to `ETH_P_IP`.
pub struct Socket {
    fd: AsyncFd<OwnedFd>,
    if_index: i32,
}

impl Socket {
    /// Opens an `AF_PACKET`/`SOCK_DGRAM` socket bound to `interface` and filtered to IPv4.
    ///
    /// # Errors
    ///
    /// Returns an error when creating or binding the packet socket fails.
    pub fn open(interface: &str) -> Result<Self> {
        let owned = socket_with(
            AddressFamily::PACKET,
            SocketType::DGRAM,
            SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
            Some(eth::IP),
        )
        .map_err(|error| Failure::Create(io::Error::from(error)))?;

        bind_device(owned.as_fd(), interface)?;

        let if_index = lookup_if_index(owned.as_fd(), interface)?;
        bind_to_interface(owned.as_fd(), if_index)?;

        let fd = AsyncFd::with_interest(owned, Interest::READABLE | Interest::WRITABLE)?;
        Ok(Self { fd, if_index })
    }

    /// Returns the interface index this socket is bound to.
    #[must_use]
    pub fn if_index(&self) -> i32 {
        self.if_index
    }

    /// Wraps an arbitrary owned file descriptor as a [`Socket`].
    #[doc(hidden)]
    pub fn from_fd(fd: OwnedFd, if_index: i32) -> Result<Self> {
        let async_fd = AsyncFd::with_interest(fd, Interest::READABLE | Interest::WRITABLE)?;
        Ok(Self {
            fd: async_fd,
            if_index,
        })
    }

    /// Sends `payload` to the link-layer destination `dst_mac` for IPv4 traffic.
    ///
    /// # Errors
    ///
    /// Returns an error when waiting for writability or sending the packet fails.
    pub async fn send_to(&self, payload: &[u8], dst_mac: [u8; ETH_ALEN]) -> Result<usize> {
        let addr = make_sockaddr_ll(self.if_index, dst_mac);
        loop {
            let mut guard = self.fd.writable().await?;
            match guard.try_io(|inner| sendto_ll(inner.get_ref(), payload, &addr)) {
                Ok(res) => return res.map_err(Failure::Io),
                Err(_would_block) => (),
            }
        }
    }

    /// Receives a single packet into `buf`, returning the number of bytes read.
    ///
    /// # Errors
    ///
    /// Returns an error when waiting for readability or receiving a packet fails.
    pub async fn recv(&self, buf: &mut [u8]) -> Result<usize> {
        loop {
            let mut guard = self.fd.readable().await?;
            match guard.try_io(|inner| recv_ll(inner.get_ref(), buf)) {
                Ok(res) => return res.map_err(Failure::Io),
                Err(_would_block) => (),
            }
        }
    }
}

fn sendto_ll(fd: &OwnedFd, payload: &[u8], addr: &SockAddrLl) -> io::Result<usize> {
    sendto(fd, payload, SendFlags::empty(), addr).map_err(io::Error::from)
}

fn recv_ll(fd: &OwnedFd, buf: &mut [u8]) -> io::Result<usize> {
    recv(fd, &mut *buf, RecvFlags::empty())
        .map(|(_, n)| n)
        .map_err(io::Error::from)
}

fn lookup_if_index(fd: BorrowedFd<'_>, interface: &str) -> Result<i32> {
    let index = netdevice::name_to_index(fd, interface).map_err(|error| Failure::Index {
        device: interface.to_owned(),
        source: io::Error::from(error),
    })?;

    i32::try_from(index).map_err(|_index_error| Failure::Index {
        device: interface.to_owned(),
        source: io::Error::new(
            io::ErrorKind::InvalidData,
            "interface index exceeds i32 range",
        ),
    })
}

fn bind_to_interface(fd: BorrowedFd<'_>, if_index: i32) -> Result<()> {
    let addr = make_sockaddr_ll(if_index, [0_u8; ETH_ALEN]);
    bind(fd, &addr).map_err(|error| Failure::BindIface {
        device: format!("if_index {if_index}"),
        source: io::Error::from(error),
    })
}

fn make_sockaddr_ll(if_index: i32, dst_mac: [u8; ETH_ALEN]) -> SockAddrLl {
    let mut address = [0_u8; 8];
    address[..ETH_ALEN].copy_from_slice(&dst_mac);

    SockAddrLl {
        family: AddressFamily::PACKET.as_raw(),
        protocol: ETH_PROTOCOL_IP,
        if_index,
        hatype: 0,
        packet_type: 0,
        hardware_addr_len: ETH_ALEN_U8,
        address,
    }
}

#[cfg(test)]
mod tests {
    use rustix::net::AddressFamily;

    use super::*;

    #[test]
    fn make_sockaddr_ll_sets_protocol_index_and_address() {
        // ARRANGE
        let mac = [1, 2, 3, 4, 5, 6];

        // ACT
        let addr = make_sockaddr_ll(7, mac);

        // ASSERT
        assert_eq!(addr.family, AddressFamily::PACKET.as_raw());
        assert_eq!(addr.protocol, ETH_PROTOCOL_IP);
        assert_eq!(addr.if_index, 7);
        assert_eq!(addr.hardware_addr_len, ETH_ALEN_U8);
        assert_eq!(&addr.address[..ETH_ALEN], &mac);
        assert_eq!(&addr.address[ETH_ALEN..], &[0, 0]);
    }

    #[test]
    fn eth_broadcast_is_all_ones() {
        // ARRANGE
        let expected = [0xff; ETH_ALEN];

        // ACT
        let broadcast = ETH_BROADCAST;

        // ASSERT
        assert_eq!(broadcast, expected);
    }
}
