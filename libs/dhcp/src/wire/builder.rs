//! `DHCPv4` message builders (RFC 2131).

use core::net::Ipv4Addr;

use super::{header, message_type, option};
use crate::{Failure, Result};

// Fixed length of a one byte DHCP option.
const MESSAGE_TYPE_LEN: usize = 3;
// Fixed length of the requested IP address option.
const REQUESTED_IP_LEN: usize = 6;
// Fixed length of the server identifier option.
const SERVER_ID_LEN: usize = 6;
// Number of option codes in the parameter request list.
const PARAM_REQUEST_COUNT: u8 = 4;
// Fixed length of the parameter request list option.
const PARAM_REQUEST_LIST_LEN: usize = 6;
const END_LEN: usize = 1;

/// `DHCPv4` REQUEST state, selecting the broadcast flag and server identifier option.
#[derive(Clone, Copy)]
pub(crate) enum Mode {
    /// SELECTING: broadcast flag set, server identifier included.
    Selecting { server: Ipv4Addr },
    /// RENEWING: broadcast flag cleared.
    Unicast,
    /// REBINDING: broadcast flag set, no server identifier.
    Broadcast,
}

/// Writes a DHCP DISCOVER message into `buf`.
///
/// Returns the written prefix of `buf`.
///
/// # Errors
///
/// Returns [`Failure::MessageTooLong`] with the required byte count when `buf` cannot hold the
/// header and the DISCOVER options.
pub(crate) fn discover(buf: &mut [u8], xid: u32, mac: [u8; 6]) -> Result<&mut [u8]> {
    let required = header::LEN
        .saturating_add(MESSAGE_TYPE_LEN)
        .saturating_add(PARAM_REQUEST_LIST_LEN)
        .saturating_add(END_LEN);
    let Some(out) = buf.get_mut(..required) else {
        return Err(Failure::MessageTooLong(required));
    };

    header::build(out, xid, mac)?;
    let mut cursor = header::LEN;
    cursor = put(
        out,
        cursor,
        &[option::MESSAGE_TYPE, 1, message_type::DISCOVER],
    )?;
    cursor = append_param_request_list(out, cursor)?;
    put(out, cursor, &[option::END])?;

    Ok(out)
}

/// Writes a DHCP REQUEST message into `buf`.
///
/// Returns the written prefix of `buf`.
///
/// # Errors
///
/// Returns [`Failure::MessageTooLong`] with the required byte count when `buf` cannot hold the
/// header and the REQUEST options.
pub(crate) fn request(
    buf: &mut [u8],
    xid: u32,
    mac: [u8; 6],
    assigned: Ipv4Addr,
    mode: Mode,
) -> Result<&mut [u8]> {
    let mut required = header::LEN
        .saturating_add(MESSAGE_TYPE_LEN)
        .saturating_add(REQUESTED_IP_LEN)
        .saturating_add(PARAM_REQUEST_LIST_LEN)
        .saturating_add(END_LEN);
    if matches!(mode, Mode::Selecting { .. }) {
        required = required.saturating_add(SERVER_ID_LEN);
    }
    let Some(out) = buf.get_mut(..required) else {
        return Err(Failure::MessageTooLong(required));
    };

    match mode {
        Mode::Selecting { .. } | Mode::Broadcast => header::build(out, xid, mac)?,
        Mode::Unicast => header::build_unicast(out, xid, mac)?,
    };

    let [first, second, third, fourth] = assigned.octets();
    let mut cursor = header::LEN;
    cursor = put(
        out,
        cursor,
        &[option::MESSAGE_TYPE, 1, message_type::REQUEST],
    )?;
    cursor = put(
        out,
        cursor,
        &[option::REQUESTED_IP, 4, first, second, third, fourth],
    )?;
    if let Mode::Selecting { server } = mode {
        let [s1, s2, s3, s4] = server.octets();
        cursor = put(out, cursor, &[option::SERVER_ID, 4, s1, s2, s3, s4])?;
    }
    cursor = append_param_request_list(out, cursor)?;
    put(out, cursor, &[option::END])?;

    Ok(out)
}

/// Writes the parameter request list option at `cursor` in `buf`.
///
/// Returns the cursor position after the option.
///
/// # Errors
///
/// Returns [`Failure::MessageTooLong`] with the required byte count when `buf` cannot hold the
/// option at `cursor`.
pub(crate) fn append_param_request_list(buf: &mut [u8], cursor: usize) -> Result<usize> {
    put(
        buf,
        cursor,
        &[
            option::PARAM_REQUEST_LIST,
            PARAM_REQUEST_COUNT,
            option::SUBNET_MASK,
            option::ROUTER,
            option::DNS_SERVER,
            option::LEASE_TIME,
        ],
    )
}

fn put(buf: &mut [u8], cursor: usize, bytes: &[u8]) -> Result<usize> {
    let end = cursor.saturating_add(bytes.len());
    let Some(slot) = buf.get_mut(cursor..end) else {
        return Err(Failure::MessageTooLong(end));
    };
    slot.copy_from_slice(bytes);

    Ok(end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MAX_MESSAGE_LEN;
    use crate::wire::field;

    const MAC: [u8; 6] = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];

    fn flags(message: &[u8]) -> u16 {
        u16::from_be_bytes([
            message.get(field::FLAGS).copied().unwrap_or(0),
            message.get(field::FLAGS + 1).copied().unwrap_or(0),
        ])
    }

    #[test]
    fn append_param_request_list_content() {
        // ARRANGE
        let mut buf = [0_u8; 16];

        // ACT
        let cursor = append_param_request_list(&mut buf, 0).expect("option fits");

        // ASSERT
        assert_eq!(cursor, PARAM_REQUEST_LIST_LEN);
        assert_eq!(buf.first(), Some(&option::PARAM_REQUEST_LIST));
        assert_eq!(buf.get(1), Some(&PARAM_REQUEST_COUNT));
        assert_eq!(buf.get(2), Some(&option::SUBNET_MASK));
        assert_eq!(buf.get(3), Some(&option::ROUTER));
        assert_eq!(buf.get(4), Some(&option::DNS_SERVER));
        assert_eq!(buf.get(5), Some(&option::LEASE_TIME));
    }

    #[test]
    fn discover_writes_message_type() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];

        // ACT
        let message = discover(&mut buf, 1, MAC).expect("message fits");

        // ASSERT
        assert_eq!(
            message.get(header::LEN..header::LEN + MESSAGE_TYPE_LEN),
            Some([option::MESSAGE_TYPE, 1, message_type::DISCOVER].as_slice())
        );
        assert_eq!(message.last(), Some(&option::END));
    }

    #[test]
    fn discover_sets_broadcast_flag() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];

        // ACT
        let message = discover(&mut buf, 1, MAC).expect("message fits");

        // ASSERT
        assert_eq!(flags(message), super::super::FLAG_BROADCAST);
    }

    #[test]
    fn request_unicast_clears_broadcast_flag() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];
        let assigned = Ipv4Addr::new(10, 0, 0, 50);

        // ACT
        let message =
            request(&mut buf, 0x1234, MAC, assigned, Mode::Unicast).expect("message fits");

        // ASSERT
        assert_eq!(flags(message), 0);
    }

    #[test]
    fn request_broadcast_sets_flag() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];
        let assigned = Ipv4Addr::new(10, 0, 0, 50);

        // ACT
        let message =
            request(&mut buf, 0x1234, MAC, assigned, Mode::Broadcast).expect("message fits");

        // ASSERT
        assert_eq!(flags(message), super::super::FLAG_BROADCAST);
    }

    #[test]
    fn request_writes_requested_ip() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];
        let assigned = Ipv4Addr::new(10, 0, 0, 50);

        // ACT
        let message =
            request(&mut buf, 0x1234, MAC, assigned, Mode::Broadcast).expect("message fits");

        // ASSERT
        let start = header::LEN + MESSAGE_TYPE_LEN;
        assert_eq!(
            message.get(start..start + REQUESTED_IP_LEN),
            Some([option::REQUESTED_IP, 4, 10, 0, 0, 50].as_slice())
        );
    }

    #[test]
    fn request_selecting_writes_server_id() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];
        let assigned = Ipv4Addr::new(10, 0, 0, 50);
        let server = Ipv4Addr::new(192, 168, 1, 1);

        // ACT
        let message = request(&mut buf, 0x1234, MAC, assigned, Mode::Selecting { server })
            .expect("message fits");

        // ASSERT
        assert_eq!(message.len(), 262);
        let start = header::LEN + MESSAGE_TYPE_LEN + REQUESTED_IP_LEN;
        assert_eq!(
            message.get(start..start + SERVER_ID_LEN),
            Some([option::SERVER_ID, 4, 192, 168, 1, 1].as_slice())
        );
    }

    #[test]
    fn request_selecting_sets_broadcast_flag() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];
        let assigned = Ipv4Addr::new(10, 0, 0, 50);
        let server = Ipv4Addr::new(192, 168, 1, 1);

        // ACT
        let message = request(&mut buf, 0x1234, MAC, assigned, Mode::Selecting { server })
            .expect("message fits");

        // ASSERT
        assert_eq!(flags(message), super::super::FLAG_BROADCAST);
    }

    #[test]
    fn request_broadcast_omits_server_id() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];
        let assigned = Ipv4Addr::new(10, 0, 0, 50);
        let message =
            request(&mut buf, 0x1234, MAC, assigned, Mode::Broadcast).expect("message fits");

        // ACT
        let after_ip = header::LEN + MESSAGE_TYPE_LEN + REQUESTED_IP_LEN;

        // ASSERT
        assert_eq!(message.len(), 256);
        assert_eq!(message.get(after_ip), Some(&option::PARAM_REQUEST_LIST));
        assert!(!message.contains(&option::SERVER_ID));
    }

    #[test]
    fn discover_rejects_short_buffer() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];
        let required = discover(&mut buf, 1, MAC).expect("message fits").len();

        // ACT
        let result = discover(&mut buf[..required - 1], 1, MAC);

        // ASSERT
        assert!(matches!(result, Err(Failure::MessageTooLong(len)) if len == required));
    }

    #[test]
    fn request_rejects_short_buffer() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];
        let assigned = Ipv4Addr::new(10, 0, 0, 50);
        let required = request(&mut buf, 1, MAC, assigned, Mode::Broadcast)
            .expect("message fits")
            .len();

        // ACT
        let result = request(&mut buf[..required - 1], 1, MAC, assigned, Mode::Broadcast);

        // ASSERT
        assert!(matches!(result, Err(Failure::MessageTooLong(len)) if len == required));
    }

    #[test]
    fn request_selecting_rejects_short_buffer() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];
        let assigned = Ipv4Addr::new(10, 0, 0, 50);
        let server = Ipv4Addr::new(192, 168, 1, 1);
        let mode = Mode::Selecting { server };
        let required = request(&mut buf, 1, MAC, assigned, mode)
            .expect("message fits")
            .len();

        // ACT
        let result = request(&mut buf[..required - 1], 1, MAC, assigned, mode);

        // ASSERT
        assert_eq!(required, 262);
        assert!(matches!(result, Err(Failure::MessageTooLong(len)) if len == required));
    }
}
