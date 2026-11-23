//! `AF_PACKET`/`SOCK_DGRAM` IPv4 socket adapter for Linux.
//!
//! Opening a socket performs every kernel side setup step and returns the owned file descriptor.
//! Sending and receiving borrow that descriptor and keep no state, so the adapter is a byte pipe
//! that never holds the descriptor across a wait.

#![warn(missing_docs)]

use std::io;
use std::os::fd::{AsFd as _, AsRawFd as _, BorrowedFd, OwnedFd};

use netmodel::Name;
use rustix::net;
use rustix::net::addr::{SocketAddrArg, SocketAddrLen, SocketAddrOpaque};

const ETH_ALEN: usize = 6;
const ETH_ALEN_U8: u8 = 6;
const ETH_PROTOCOL_IP: u16 = 0x0800_u16.to_be();
const SOCK_ADDR_LL_LEN: SocketAddrLen = 20;
const IF_NAME_MAX: usize = 16;

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

// SAFETY: `SockAddrLl` is `repr(C)` and matches the kernel ABI for `sockaddr_ll`, and
// `SOCK_ADDR_LL_LEN` is the exact length the kernel expects for `AF_PACKET` calls.
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

/// Opens an `AF_PACKET`/`SOCK_DGRAM` socket bound to `link` and filtered to IPv4.
///
/// The socket is non blocking and close on exec, so waiting for readiness is the caller's job.
///
/// # Errors
///
/// Returns an error when socket creation, `SO_BINDTODEVICE`, the interface index lookup, or the
/// interface bind fails.
pub fn open(link: &Name) -> io::Result<OwnedFd> {
    let fd = net::socket_with(
        net::AddressFamily::PACKET,
        net::SocketType::DGRAM,
        net::SocketFlags::NONBLOCK | net::SocketFlags::CLOEXEC,
        Some(net::eth::IP),
    )
    .map_err(io::Error::from)?;

    bind_device(fd.as_fd(), link.as_str())?;

    let index =
        net::netdevice::name_to_index(fd.as_fd(), link.as_str()).map_err(io::Error::from)?;
    let index = i32::try_from(index).map_err(|_error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "interface index exceeds i32 range",
        )
    })?;

    net::bind(fd.as_fd(), &make_sockaddr_ll(index, [0_u8; ETH_ALEN])).map_err(io::Error::from)?;

    Ok(fd)
}

/// Sends `frame` to the link-layer destination `dst` on the socket's bound interface.
///
/// The adapter stores no state, so this calls `getsockname` to recover the interface index that
/// `open` bound. That costs one extra syscall per frame.
///
/// # Errors
///
/// Returns an error when the descriptor is not a bound `AF_PACKET` socket or the send fails.
/// [`io::ErrorKind::WouldBlock`] is a normal result for a non blocking socket.
pub fn send(fd: BorrowedFd<'_>, frame: &[u8], dst: [u8; ETH_ALEN]) -> io::Result<usize> {
    let index = bound_if_index(fd)?;
    net::sendto(
        fd,
        frame,
        net::SendFlags::empty(),
        &make_sockaddr_ll(index, dst),
    )
    .map_err(io::Error::from)
}

/// Receives one frame into `buf`, returning the number of bytes read.
///
/// # Errors
///
/// Returns an error when the receive fails. [`io::ErrorKind::WouldBlock`] is a normal result for a
/// non blocking socket.
pub fn recv(fd: BorrowedFd<'_>, buf: &mut [u8]) -> io::Result<usize> {
    net::recv(fd, buf, net::RecvFlags::empty())
        .map(|(_flags, len)| len)
        .map_err(io::Error::from)
}

// `bind_device` and `copy_device_name` are duplicated in the `icmpv6-linux` adapter. Sharing them
// needs a support crate that is outside this lane, so the duplication is follow-up tech debt.
fn bind_device(fd: BorrowedFd<'_>, device: &str) -> io::Result<()> {
    let mut name = [0_u8; IF_NAME_MAX];
    let length = copy_device_name(&mut name, device)?;
    let length = libc::socklen_t::try_from(length).map_err(|_error| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "device name length exceeds socklen_t range",
        )
    })?;

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
        Err(io::Error::last_os_error())
    }
}

fn copy_device_name(buffer: &mut [u8; IF_NAME_MAX], device: &str) -> io::Result<usize> {
    let bytes = device.as_bytes();
    let length = bytes.len().saturating_add(1);
    if bytes.is_empty() || length > buffer.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "device name does not fit the SO_BINDTODEVICE buffer",
        ));
    }

    buffer.fill(0);
    let (name, _terminator) = buffer.split_at_mut(bytes.len());
    name.copy_from_slice(bytes);

    Ok(length)
}

// The adapter keeps no state between calls, so the interface index that `open` bound is recovered
// from the socket address instead of being cached next to the descriptor.
fn bound_if_index(fd: BorrowedFd<'_>) -> io::Result<i32> {
    let addr = net::getsockname(fd).map_err(io::Error::from)?;
    if addr.address_family() != net::AddressFamily::PACKET || addr.addr_len() < SOCK_ADDR_LL_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "socket is not bound to an AF_PACKET device",
        ));
    }

    // SAFETY: `addr` reports the `AF_PACKET` family and carries at least one full `sockaddr_ll`,
    // which the kernel wrote using that layout.
    let bound = unsafe { core::ptr::read(addr.as_ptr().cast::<SockAddrLl>()) };

    Ok(bound.if_index)
}

fn make_sockaddr_ll(if_index: i32, dst: [u8; ETH_ALEN]) -> SockAddrLl {
    let mut address = [0_u8; 8];
    address[..ETH_ALEN].copy_from_slice(&dst);

    SockAddrLl {
        family: net::AddressFamily::PACKET.as_raw(),
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
    use core::mem::size_of;
    use std::os::unix::net::UnixStream;

    use super::*;

    #[test]
    fn sockaddr_ll_matches_kernel_length() {
        // ARRANGE
        let expected = size_of::<SockAddrLl>();

        // ACT
        let length = usize::try_from(SOCK_ADDR_LL_LEN).expect("sockaddr_ll length fits usize");

        // ASSERT
        assert_eq!(length, expected);
    }

    #[test]
    fn make_sockaddr_ll_sets_protocol_index_and_address() {
        // ARRANGE
        let mac = [1, 2, 3, 4, 5, 6];

        // ACT
        let addr = make_sockaddr_ll(7, mac);

        // ASSERT
        assert_eq!(addr.family, net::AddressFamily::PACKET.as_raw());
        assert_eq!(addr.protocol, ETH_PROTOCOL_IP);
        assert_eq!(addr.if_index, 7_i32);
        assert_eq!(addr.hardware_addr_len, ETH_ALEN_U8);
        assert_eq!(addr.address[..ETH_ALEN], mac);
        assert_eq!(addr.address[ETH_ALEN..], [0, 0]);
    }

    #[test]
    fn copy_device_name_rejects_names_that_do_not_fit() {
        // ARRANGE
        let mut buffer = [0_u8; IF_NAME_MAX];
        let too_long = "a".repeat(IF_NAME_MAX);

        // ACT
        let result = copy_device_name(&mut buffer, &too_long);

        // ASSERT
        assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::InvalidInput));
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
    fn send_rejects_descriptor_that_is_not_a_packet_socket() {
        // ARRANGE
        let (socket, _peer) = UnixStream::pair().expect("unix socket pair should open");
        let frame = [0_u8; 4];

        // ACT
        let result = send(socket.as_fd(), &frame, [0xff; ETH_ALEN]);

        // ASSERT
        assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::InvalidInput));
    }
}
