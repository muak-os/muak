//! Async raw `ICMPv6` socket bound to a single interface.

use core::mem::size_of;
use core::net::{Ipv6Addr, SocketAddrV6};
use core::result::Result as CoreResult;
use std::io;
use std::os::fd::{AsFd, AsRawFd as _, OwnedFd};

use rustix::net::ipproto::ICMPV6;
use rustix::net::{
    AddressFamily, RecvFlags, SendFlags, SocketAddrAny, SocketFlags, SocketType, netdevice,
    recvfrom, sendto, socket_with,
};
use thiserror::Error;
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;

use crate::socket::{Failure as SocketFailure, bind_device};

const ICMP6_FILTER: libc::c_int = 1;
const FILTER_WORDS: usize = 8;
const BLOCK_ALL: u32 = u32::MAX;

/// Raw `ICMPv6` socket operation failures.
#[derive(Debug, Error)]
pub enum Failure {
    /// Failed to create a raw `ICMPv6` socket.
    #[error("failed to create raw ICMPv6 socket: {0}")]
    Create(#[source] io::Error),
    /// Failed to bind the raw socket to its interface.
    #[error("failed to bind raw ICMPv6 socket via SO_BINDTODEVICE: {0}")]
    BindDevice(#[from] SocketFailure),
    /// Failed to look up the interface index.
    #[error("failed to look up interface index for {device}: {source}")]
    Index {
        /// Device name.
        device: String,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// Failed to apply the `ICMP6_FILTER` socket option.
    #[error("failed to apply ICMP6_FILTER: {0}")]
    Filter(#[source] io::Error),
    /// I/O error on the raw socket.
    #[error("I/O error on raw ICMPv6 socket: {0}")]
    Io(#[from] io::Error),
}

/// Raw `ICMPv6` socket result type.
pub type Result<T> = CoreResult<T, Failure>;

/// Async raw `ICMPv6` socket bound to a specific interface.
pub struct Socket {
    fd: AsyncFd<OwnedFd>,
    ifindex: u32,
}

impl Socket {
    /// Opens a raw `ICMPv6` socket bound to `interface` and filtered to `allowed_types`.
    ///
    /// # Errors
    ///
    /// Returns an error when creating, binding, or configuring the socket fails.
    pub fn open(interface: &str, allowed_types: &[u8]) -> Result<Self> {
        let owned = socket_with(
            AddressFamily::INET6,
            SocketType::RAW,
            SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
            Some(ICMPV6),
        )
        .map_err(|error| Failure::Create(io::Error::from(error)))?;

        bind_device(owned.as_fd(), interface)?;

        let ifindex =
            netdevice::name_to_index(owned.as_fd(), interface).map_err(|error| Failure::Index {
                device: interface.to_owned(),
                source: io::Error::from(error),
            })?;

        apply_filter(owned.as_fd(), &Filter::allowing(allowed_types))?;

        // SAFETY: `owned` uniquely owns the file descriptor and keeps it open for the
        // lifetime of the returned `AsyncFd`, so `as_raw_fd` stays valid and unchanged.
        let fd = unsafe {
            AsyncFd::register_with_interest(owned, Interest::READABLE | Interest::WRITABLE)
        }
        .map_err(io::Error::from)?;

        Ok(Self { fd, ifindex })
    }

    /// Returns the interface index this socket is bound to.
    #[must_use = "callers need the interface index to scope datagrams"]
    pub const fn if_index(&self) -> u32 {
        self.ifindex
    }

    /// Sends `payload` to `dst` scoped to this socket's interface.
    ///
    /// # Errors
    ///
    /// Returns an error when waiting for writability or sending the datagram fails.
    pub async fn send_to(&self, dst: Ipv6Addr, payload: &[u8]) -> Result<usize> {
        let addr = SocketAddrV6::new(dst, 0, 0, self.ifindex);
        loop {
            let mut guard = self.fd.writable().await?;
            match guard.try_io(|inner| send_datagram(inner.get_ref(), payload, &addr)) {
                Ok(result) => return result.map_err(Failure::Io),
                Err(_would_block) => (),
            }
        }
    }

    /// Receives one datagram into `buf`, returning its length and IPv6 source.
    ///
    /// # Errors
    ///
    /// Returns an error when waiting for readability or receiving the datagram fails.
    pub async fn recv_from(&self, buf: &mut [u8]) -> Result<(usize, Option<Ipv6Addr>)> {
        loop {
            let mut guard = self.fd.readable().await?;
            match guard.try_io(|inner| recv_datagram(inner.get_ref(), &mut *buf)) {
                Ok(result) => return result.map_err(Failure::Io),
                Err(_would_block) => (),
            }
        }
    }
}

#[repr(C)]
struct Filter {
    data: [u32; FILTER_WORDS],
}

impl Filter {
    fn allowing(allowed_types: &[u8]) -> Self {
        let mut filter = Self {
            data: [BLOCK_ALL; FILTER_WORDS],
        };
        for &icmp_type in allowed_types {
            filter.allow(icmp_type);
        }
        filter
    }

    fn allow(&mut self, icmp_type: u8) {
        let index = usize::from(icmp_type);
        let Some(word) = self.data.get_mut(index >> 5_usize) else {
            return;
        };
        *word &= !(1_u32 << (index & 31_usize));
    }
}

fn send_datagram(fd: &OwnedFd, payload: &[u8], addr: &SocketAddrV6) -> io::Result<usize> {
    sendto(fd, payload, SendFlags::empty(), addr).map_err(io::Error::from)
}

fn recv_datagram(fd: &OwnedFd, buf: &mut [u8]) -> io::Result<(usize, Option<Ipv6Addr>)> {
    recvfrom(fd, &mut *buf, RecvFlags::empty())
        .map(|(_, len, addr)| (len, source_address(addr)))
        .map_err(io::Error::from)
}

fn apply_filter<Fd: AsFd>(fd: Fd, filter: &Filter) -> Result<()> {
    let filter_len = libc::socklen_t::try_from(size_of::<Filter>()).unwrap_or(0);
    // SAFETY: We pass valid pointers and sizes for the filter struct, fd is valid
    let result = unsafe {
        libc::setsockopt(
            fd.as_fd().as_raw_fd(),
            libc::IPPROTO_ICMPV6,
            ICMP6_FILTER,
            core::ptr::from_ref(filter).cast::<libc::c_void>(),
            filter_len,
        )
    };

    if result == 0 {
        Ok(())
    } else {
        Err(Failure::Filter(io::Error::last_os_error()))
    }
}

fn source_address(addr: Option<SocketAddrAny>) -> Option<Ipv6Addr> {
    let addr = addr?;
    if addr.address_family() != AddressFamily::INET6 {
        return None;
    }
    SocketAddrV6::try_from(addr)
        .ok()
        .map(|sockaddr| *sockaddr.ip())
}

#[cfg(test)]
mod tests {
    use std::os::fd::BorrowedFd;

    use super::*;

    const ROUTER_ADVERTISEMENT: u8 = 134;

    #[test]
    fn filter_allows_listed_types() {
        // ARRANGE
        let allowed = [ROUTER_ADVERTISEMENT];

        // ACT
        let filter = Filter::allowing(&allowed);

        // ASSERT
        let index = usize::from(ROUTER_ADVERTISEMENT);
        let word = filter
            .data
            .get(index >> 5_usize)
            .copied()
            .unwrap_or(BLOCK_ALL);
        assert_eq!(word & (1_u32 << (index & 31_usize)), 0);
    }

    #[test]
    fn filter_blocks_unlisted_types() {
        // ARRANGE
        let allowed = [ROUTER_ADVERTISEMENT];

        // ACT
        let filter = Filter::allowing(&allowed);

        // ASSERT
        let icmp_types = [0_u8, 1, 128, 129, 133, 135, 136];
        for icmp_type in icmp_types
            .into_iter()
            .filter(|icmp| *icmp != ROUTER_ADVERTISEMENT)
        {
            let index = usize::from(icmp_type);
            let word = filter.data.get(index >> 5_usize).copied().unwrap_or(0);
            assert_ne!(
                word & (1_u32 << (index & 31_usize)),
                0,
                "type {icmp_type} should be blocked"
            );
        }
    }

    #[test]
    fn filter_starts_with_everything_blocked() {
        // ARRANGE
        let allowed = [ROUTER_ADVERTISEMENT];

        // ACT
        let filter = Filter::allowing(&allowed);

        // ASSERT
        let pass_count = (0_u16..256)
            .filter(|&icmp_type| {
                let index = usize::from(icmp_type);
                let word = filter.data.get(index >> 5_usize).copied().unwrap_or(0);
                word & (1_u32 << (index & 31_usize)) == 0
            })
            .count();

        assert_eq!(pass_count, 1, "only the listed type should pass");
    }

    #[test]
    fn apply_filter_fails_on_invalid_fd() {
        // ARRANGE
        let filter = Filter::allowing(&[ROUTER_ADVERTISEMENT]);

        // ACT
        // SAFETY: fd 9999 is virtually guaranteed to be invalid (EBADF)
        let result = unsafe {
            let bad_fd = BorrowedFd::borrow_raw(9999);
            apply_filter(bad_fd, &filter)
        };

        // ASSERT
        assert!(matches!(result, Err(Failure::Filter(_))));
    }
}
