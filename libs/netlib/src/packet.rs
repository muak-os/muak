//! Async `AF_PACKET`/`SOCK_DGRAM` raw socket bound to a single interface.

use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};

use rustix::net::addr::{SocketAddrArg, SocketAddrLen, SocketAddrOpaque};
use rustix::net::{
    AddressFamily, RecvFlags, SendFlags, SocketFlags, SocketType, eth, netdevice, recv, socket_with,
};
use thiserror::Error;
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;

use crate::socket::bind_device;

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to create AF_PACKET socket: {0}")]
    Create(#[source] io::Error),
    #[error("failed to bind AF_PACKET socket to interface {device}: {source}")]
    BindIface {
        device: String,
        #[source]
        source: io::Error,
    },
    #[error("failed to bind AF_PACKET socket via SO_BINDTODEVICE: {0}")]
    BindDevice(#[from] crate::socket::Error),
    #[error("failed to look up interface index for {device}: {source}")]
    Index {
        device: String,
        #[source]
        source: io::Error,
    },
    #[error("I/O error on packet socket: {0}")]
    Io(#[from] io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

const ETH_ALEN: usize = 6;
/// Standard link-layer broadcast address.
pub const ETH_BROADCAST: [u8; ETH_ALEN] = [0xff; ETH_ALEN];

/// Stable kernel UAPI `struct sockaddr_ll` from `<linux/if_packet.h>`.
#[repr(C)]
struct SockAddrLl {
    sll_family: u16,
    sll_protocol: u16,
    sll_ifindex: i32,
    sll_hatype: u16,
    sll_pkttype: u8,
    sll_halen: u8,
    sll_addr: [u8; 8],
}

// SAFETY: `SockAddrLl` is `repr(C)` matching the kernel ABI for `sockaddr_ll`,
// and its size is the exact length the kernel expects for `AF_PACKET` syscalls.
unsafe impl SocketAddrArg for SockAddrLl {
    unsafe fn with_sockaddr<R>(
        &self,
        f: impl FnOnce(*const SocketAddrOpaque, SocketAddrLen) -> R,
    ) -> R {
        f(
            std::ptr::from_ref(self).cast::<SocketAddrOpaque>(),
            size_of::<SockAddrLl>() as SocketAddrLen,
        )
    }
}

/// Async DHCP raw socket bound to a specific interface, filtered to ETH_P_IP.
pub struct PacketSocket {
    fd: AsyncFd<OwnedFd>,
    if_index: i32,
}

impl PacketSocket {
    /// Opens an `AF_PACKET`/`SOCK_DGRAM` socket bound to `interface` and filtered to IPv4.
    pub fn open(interface: &str) -> Result<Self> {
        let owned = socket_with(
            AddressFamily::PACKET,
            SocketType::DGRAM,
            SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
            Some(eth::IP),
        )
        .map_err(|e| Error::Create(io::Error::from(e)))?;

        bind_device(owned.as_fd(), interface)?;

        let if_index = lookup_if_index(owned.as_fd(), interface)?;
        bind_to_interface(owned.as_fd(), if_index)?;

        let fd = AsyncFd::with_interest(owned, Interest::READABLE | Interest::WRITABLE)?;
        Ok(Self { fd, if_index })
    }

    /// Returns the interface index this socket is bound to.
    pub fn if_index(&self) -> i32 {
        self.if_index
    }

    /// Wraps an arbitrary owned file descriptor as a `PacketSocket`.
    #[doc(hidden)]
    pub fn from_fd(fd: OwnedFd, if_index: i32) -> Result<Self> {
        let async_fd = AsyncFd::with_interest(fd, Interest::READABLE | Interest::WRITABLE)?;
        Ok(Self {
            fd: async_fd,
            if_index,
        })
    }

    /// Sends `payload` to the link-layer destination `dst_mac` for IPv4 traffic.
    pub async fn send_to(&self, payload: &[u8], dst_mac: [u8; ETH_ALEN]) -> Result<usize> {
        let addr = make_sockaddr_ll(self.if_index, dst_mac);
        loop {
            let mut guard = self.fd.writable().await?;
            match guard.try_io(|inner| sendto_ll(inner.get_ref(), payload, &addr)) {
                Ok(res) => return res.map_err(Error::Io),
                Err(_would_block) => continue,
            }
        }
    }

    /// Receives a single packet into `buf`, returning the number of bytes read.
    pub async fn recv(&self, buf: &mut [u8]) -> Result<usize> {
        loop {
            let mut guard = self.fd.readable().await?;
            match guard.try_io(|inner| recv_ll(inner.get_ref(), buf)) {
                Ok(res) => return res.map_err(Error::Io),
                Err(_would_block) => continue,
            }
        }
    }
}

fn sendto_ll(fd: &OwnedFd, payload: &[u8], addr: &SockAddrLl) -> io::Result<usize> {
    rustix::net::sendto(fd, payload, SendFlags::empty(), addr).map_err(io::Error::from)
}

fn recv_ll(fd: &OwnedFd, buf: &mut [u8]) -> io::Result<usize> {
    recv(fd, &mut *buf, RecvFlags::empty())
        .map(|(_, n)| n)
        .map_err(io::Error::from)
}

fn lookup_if_index(fd: BorrowedFd<'_>, interface: &str) -> Result<i32> {
    netdevice::name_to_index(fd, interface)
        .map(|idx| idx as i32)
        .map_err(|e| Error::Index {
            device: interface.to_string(),
            source: io::Error::from(e),
        })
}

fn bind_to_interface(fd: BorrowedFd<'_>, if_index: i32) -> Result<()> {
    let addr = make_sockaddr_ll(if_index, [0u8; ETH_ALEN]);
    rustix::net::bind(fd, &addr).map_err(|e| Error::BindIface {
        device: format!("if_index {if_index}"),
        source: io::Error::from(e),
    })
}

fn make_sockaddr_ll(if_index: i32, dst_mac: [u8; ETH_ALEN]) -> SockAddrLl {
    let mut sll_addr = [0u8; 8];
    sll_addr[..ETH_ALEN].copy_from_slice(&dst_mac);
    SockAddrLl {
        sll_family: AddressFamily::PACKET.as_raw(),
        sll_protocol: eth::IP.as_raw().get() as u16,
        sll_ifindex: if_index,
        sll_hatype: 0,
        sll_pkttype: 0,
        sll_halen: ETH_ALEN as u8,
        sll_addr,
    }
}
