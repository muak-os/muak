//! Raw `ICMPv6` socket adapter that owns the descriptor and keeps no state between sends and receives.

#![warn(missing_docs)]

pub mod error;

use core::mem::size_of;
use core::net::{Ipv6Addr, SocketAddrV6};
use std::io;
use std::os::fd::{AsFd as _, AsRawFd as _, BorrowedFd, OwnedFd};

use netmodel::Name;
use rustix::net;
use rustix::net::ipproto;

use crate::error::{Icmpv6Error, Result};

const ICMP6_FILTER: libc::c_int = 1;
const FILTER_WORDS: usize = 8;
const ALLOW_ALL: u32 = 0;
const IF_NAME_MAX: usize = 16;

#[repr(C)]
struct Filter {
    data: [u32; FILTER_WORDS],
}

impl Filter {
    const fn allow_all() -> Self {
        Self {
            data: [ALLOW_ALL; FILTER_WORDS],
        }
    }
}

/// Opens a raw `ICMPv6` socket bound to `link`.
///
/// # Errors
///
/// Returns [`Icmpv6Error::InvalidDevice`] when the link name cannot be applied, [`Icmpv6Error::InvalidDeviceLength`] when
/// the name length does not fit the `socklen_t` range, [`Icmpv6Error::InvalidFilter`] when the filter length does not fit
/// the `socklen_t` range, and [`Icmpv6Error::Io`] when socket creation, `SO_BINDTODEVICE`, or the `ICMP6_FILTER` option
/// fails.
pub fn open(link: &Name) -> Result<OwnedFd> {
    let fd = net::socket_with(
        net::AddressFamily::INET6,
        net::SocketType::RAW,
        net::SocketFlags::NONBLOCK | net::SocketFlags::CLOEXEC,
        Some(ipproto::ICMPV6),
    )
    .map_err(io::Error::from)?;

    bind_device(fd.as_fd(), link.as_str())?;
    apply_filter(fd.as_fd(), &Filter::allow_all())?;

    Ok(fd)
}

/// Sends `payload` to `dst` scoped to the socket's bound interface.
///
/// # Errors
///
/// Returns [`Icmpv6Error::Io`] when the send fails. [`std::io::ErrorKind::WouldBlock`] is a normal result for a non
/// blocking socket.
pub fn send(fd: BorrowedFd<'_>, dst: Ipv6Addr, payload: &[u8]) -> Result<usize> {
    let addr = SocketAddrV6::new(dst, 0, 0, 0);

    let length =
        net::sendto(fd, payload, net::SendFlags::empty(), &addr).map_err(io::Error::from)?;

    Ok(length)
}

/// Receives one datagram into `buf`, returning its length and its IPv6 source.
///
/// # Errors
///
/// Returns [`Icmpv6Error::Io`] when the receive fails. [`std::io::ErrorKind::WouldBlock`] is a normal result for a non
/// blocking socket.
pub fn recv(fd: BorrowedFd<'_>, buf: &mut [u8]) -> Result<(usize, Option<Ipv6Addr>)> {
    let (_flags, length, addr) =
        net::recvfrom(fd, buf, net::RecvFlags::empty()).map_err(io::Error::from)?;

    Ok((length, source_address(addr)))
}

fn bind_device(fd: BorrowedFd<'_>, device: &str) -> Result<()> {
    let mut name = [0_u8; IF_NAME_MAX];
    let length = copy_device_name(&mut name, device)?;
    let length =
        libc::socklen_t::try_from(length).map_err(|_error| Icmpv6Error::InvalidDeviceLength)?;

    // SAFETY: `name` holds a NUL terminated device name of `length` bytes and `fd` is a socket.
    let result = unsafe {
        libc::setsockopt(
            fd.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_BINDTODEVICE,
            name.as_ptr().cast::<libc::c_void>(),
            length,
        )
    };

    if result == 0 {
        Ok(())
    } else {
        Err(Icmpv6Error::Io(io::Error::last_os_error()))
    }
}

fn copy_device_name(buffer: &mut [u8; IF_NAME_MAX], device: &str) -> Result<usize> {
    let bytes = device.as_bytes();
    let length = bytes.len().saturating_add(1);
    if bytes.is_empty() || length > buffer.len() {
        return Err(Icmpv6Error::InvalidDevice);
    }

    buffer.fill(0);
    let (name, _terminator) = buffer.split_at_mut(bytes.len());
    name.copy_from_slice(bytes);

    Ok(length)
}

fn apply_filter(fd: BorrowedFd<'_>, filter: &Filter) -> Result<()> {
    let length = libc::socklen_t::try_from(size_of::<Filter>())
        .map_err(|_error| Icmpv6Error::InvalidFilter)?;

    // SAFETY: `filter` is a fully initialized `icmp6_filter` of `length` bytes and `fd` is a socket.
    let result = unsafe {
        libc::setsockopt(
            fd.as_raw_fd(),
            libc::IPPROTO_ICMPV6,
            ICMP6_FILTER,
            core::ptr::from_ref(filter).cast::<libc::c_void>(),
            length,
        )
    };

    if result == 0 {
        Ok(())
    } else {
        Err(Icmpv6Error::Io(io::Error::last_os_error()))
    }
}

fn source_address(addr: Option<net::SocketAddrAny>) -> Option<Ipv6Addr> {
    let addr = addr?;
    if addr.address_family() != net::AddressFamily::INET6 {
        return None;
    }

    SocketAddrV6::try_from(addr)
        .ok()
        .map(|sockaddr| *sockaddr.ip())
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;

    use super::*;

    #[test]
    fn filter_allows_every_type() {
        // ARRANGE
        let filter = Filter::allow_all();

        // ACT
        let blocked = filter
            .data
            .iter()
            .filter(|word| **word != ALLOW_ALL)
            .count();

        // ASSERT
        assert_eq!(blocked, 0);
    }

    #[test]
    fn filter_matches_icmp6_filter_abi() {
        // ARRANGE
        let expected = 32;

        // ACT
        let length = size_of::<Filter>();

        // ASSERT
        assert_eq!(length, expected);
    }

    #[test]
    fn copy_device_name_rejects_names_that_do_not_fit() {
        // ARRANGE
        let mut buffer = [0_u8; IF_NAME_MAX];
        let too_long = "a".repeat(IF_NAME_MAX);

        // ACT
        let result = copy_device_name(&mut buffer, &too_long);

        // ASSERT
        assert!(matches!(result, Err(Icmpv6Error::InvalidDevice)));
    }

    #[test]
    fn copy_device_name_nul_terminates() {
        // ARRANGE
        let mut buffer = [0xff_u8; IF_NAME_MAX];

        // ACT
        let length = copy_device_name(&mut buffer, "eth0").expect("valid device name");

        // ASSERT
        assert_eq!(length, 5);
        assert_eq!(buffer[..5], *b"eth0\0");
        assert_eq!(buffer[5], 0);
    }

    #[test]
    fn apply_filter_rejects_non_socket_descriptor() {
        // ARRANGE
        let (socket, _peer) = UnixStream::pair().expect("unix socket pair should open");

        // ACT
        let result = apply_filter(socket.as_fd(), &Filter::allow_all());

        // ASSERT
        assert!(matches!(result, Err(Icmpv6Error::Io(_))));
    }

    #[test]
    fn source_address_rejects_foreign_family() {
        // ARRANGE
        let (socket, _peer) = UnixStream::pair().expect("unix socket pair should open");

        // ACT
        let addr = net::getsockname(socket.as_fd()).expect("getsockname should succeed");

        // ASSERT
        assert_eq!(source_address(Some(addr)), None);
        assert_eq!(source_address(None), None);
    }
}
