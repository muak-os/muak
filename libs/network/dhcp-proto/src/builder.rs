//! `DHCPv4` message builders (RFC 2131).

use core::net::Ipv4Addr;

use super::{header, message_type, option};
use crate::{FLAG_BROADCAST, Failure, Result};

const MESSAGE_TYPE_LEN: usize = 3;
const REQUESTED_IP_LEN: usize = 6;
const SERVER_ID_LEN: usize = 6;
const PARAM_REQUEST_COUNT: u8 = 4;
const PARAM_REQUEST_LIST_LEN: usize = 6;
const END_LEN: usize = 1;

/// `DHCPv4` REQUEST state, selecting how the client identifies its address.
#[derive(Clone, Copy)]
pub enum Mode {
    /// SELECTING: option 50 and the server identifier, `ciaddr` zero, broadcast flag set.
    Selecting {
        /// Server that offered the lease.
        server: Ipv4Addr,
    },
    /// RENEWING: `ciaddr` holds the assigned address, no option 50, no server identifier, broadcast flag cleared.
    Renew,
    /// REBINDING: `ciaddr` holds the assigned address, no option 50, no server identifier, broadcast flag set.
    Rebind,
}

/// Writes a DHCP DISCOVER message with the caller supplied transaction ID into `buf`.
///
/// # Errors
///
/// Returns [`Failure::MessageTooLong`] with the required byte count when `buf` cannot hold the header and the DISCOVER
/// options.
pub fn discover(buf: &mut [u8], xid: u32, mac: [u8; 6]) -> Result<&mut [u8]> {
    let required = header::LEN
        .saturating_add(MESSAGE_TYPE_LEN)
        .saturating_add(PARAM_REQUEST_LIST_LEN)
        .saturating_add(END_LEN);
    let Some(out) = buf.get_mut(..required) else {
        return Err(Failure::MessageTooLong {
            required,
            capacity: buf.len(),
        });
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

/// Writes a DHCP REQUEST message with the caller supplied transaction ID into `buf`.
///
/// # Errors
///
/// Returns [`Failure::MessageTooLong`] with the required byte count when `buf` cannot hold the header and the REQUEST
/// options.
pub fn request(
    buf: &mut [u8],
    xid: u32,
    mac: [u8; 6],
    assigned: Ipv4Addr,
    mode: Mode,
) -> Result<&mut [u8]> {
    let mut required = header::LEN
        .saturating_add(MESSAGE_TYPE_LEN)
        .saturating_add(PARAM_REQUEST_LIST_LEN)
        .saturating_add(END_LEN);
    if matches!(mode, Mode::Selecting { .. }) {
        required = required
            .saturating_add(REQUESTED_IP_LEN)
            .saturating_add(SERVER_ID_LEN);
    }
    let Some(out) = buf.get_mut(..required) else {
        return Err(Failure::MessageTooLong {
            required,
            capacity: buf.len(),
        });
    };

    match mode {
        Mode::Selecting { .. } => header::build(out, xid, mac)?,
        Mode::Renew => header::build_with(out, xid, mac, assigned, 0)?,
        Mode::Rebind => header::build_with(out, xid, mac, assigned, FLAG_BROADCAST)?,
    };

    let mut cursor = header::LEN;
    cursor = put(
        out,
        cursor,
        &[option::MESSAGE_TYPE, 1, message_type::REQUEST],
    )?;
    if let Mode::Selecting { server } = mode {
        let [first, second, third, fourth] = assigned.octets();
        cursor = put(
            out,
            cursor,
            &[option::REQUESTED_IP, 4, first, second, third, fourth],
        )?;
        let [s1, s2, s3, s4] = server.octets();
        cursor = put(out, cursor, &[option::SERVER_ID, 4, s1, s2, s3, s4])?;
    }
    cursor = append_param_request_list(out, cursor)?;
    put(out, cursor, &[option::END])?;

    Ok(out)
}

/// Writes the parameter request list option at `cursor` in `buf`.
///
/// # Errors
///
/// Returns [`Failure::MessageTooLong`] with the required byte count when `buf` cannot hold the option at `cursor`.
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
        return Err(Failure::MessageTooLong {
            required: end,
            capacity: buf.len(),
        });
    };
    slot.copy_from_slice(bytes);

    Ok(end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field;

    const MAX_MESSAGE_LEN: usize = 1500;
    const MAC: [u8; 6] = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];

    fn flags(message: &[u8]) -> u16 {
        u16::from_be_bytes([
            message.get(field::FLAGS).copied().unwrap_or(0),
            message.get(field::FLAGS + 1).copied().unwrap_or(0),
        ])
    }

    fn ciaddr(message: &[u8]) -> Ipv4Addr {
        let mut octets = [0_u8; 4];
        octets.copy_from_slice(
            message
                .get(field::CIADDR..field::CIADDR + 4)
                .expect("message carries a ciaddr field"),
        );

        Ipv4Addr::from(octets)
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
    fn discover_writes_given_xid() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];

        // ACT
        let message = discover(&mut buf, 0x0BAD_F00D, MAC).expect("message fits");

        // ASSERT
        assert_eq!(
            message.get(field::XID..field::XID + 4),
            Some(&0x0BAD_F00D_u32.to_be_bytes()[..])
        );
    }

    #[test]
    fn discover_honours_distinct_xids() {
        // ARRANGE
        let mut first_buf = [0_u8; MAX_MESSAGE_LEN];
        let mut second_buf = [0_u8; MAX_MESSAGE_LEN];

        // ACT
        let first = discover(&mut first_buf, 1, MAC).expect("message fits");
        let second = discover(&mut second_buf, 2, MAC).expect("message fits");

        // ASSERT
        assert_ne!(
            first.get(field::XID..field::XID + 4),
            second.get(field::XID..field::XID + 4)
        );
    }

    #[test]
    fn request_writes_given_xid() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];
        let assigned = Ipv4Addr::new(10, 0, 0, 50);

        // ACT
        let message =
            request(&mut buf, 0xDEAD_BEEF, MAC, assigned, Mode::Renew).expect("message fits");

        // ASSERT
        assert_eq!(
            message.get(field::XID..field::XID + 4),
            Some(&0xDEAD_BEEF_u32.to_be_bytes()[..])
        );
    }

    #[test]
    fn discover_sets_broadcast_flag() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];

        // ACT
        let message = discover(&mut buf, 1, MAC).expect("message fits");

        // ASSERT
        assert_eq!(flags(message), FLAG_BROADCAST);
    }

    #[test]
    fn request_renew_clears_broadcast_flag() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];
        let assigned = Ipv4Addr::new(10, 0, 0, 50);

        // ACT
        let message = request(&mut buf, 0x1234, MAC, assigned, Mode::Renew).expect("message fits");

        // ASSERT
        assert_eq!(flags(message), 0);
    }

    #[test]
    fn request_rebind_sets_broadcast_flag() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];
        let assigned = Ipv4Addr::new(10, 0, 0, 50);

        // ACT
        let message = request(&mut buf, 0x1234, MAC, assigned, Mode::Rebind).expect("message fits");

        // ASSERT
        assert_eq!(flags(message), FLAG_BROADCAST);
    }

    #[test]
    fn request_renew_writes_ciaddr() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];
        let assigned = Ipv4Addr::new(10, 0, 0, 50);

        // ACT
        let message = request(&mut buf, 0x1234, MAC, assigned, Mode::Renew).expect("message fits");

        // ASSERT
        assert_eq!(ciaddr(message), assigned);
    }

    #[test]
    fn request_rebind_writes_ciaddr() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];
        let assigned = Ipv4Addr::new(10, 0, 0, 50);

        // ACT
        let message = request(&mut buf, 0x1234, MAC, assigned, Mode::Rebind).expect("message fits");

        // ASSERT
        assert_eq!(ciaddr(message), assigned);
    }

    #[test]
    fn request_selecting_writes_requested_ip() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];
        let assigned = Ipv4Addr::new(10, 0, 0, 50);
        let server = Ipv4Addr::new(192, 168, 1, 1);

        // ACT
        let message = request(&mut buf, 0x1234, MAC, assigned, Mode::Selecting { server })
            .expect("message fits");

        // ASSERT
        let start = header::LEN + MESSAGE_TYPE_LEN;
        assert_eq!(
            message.get(start..start + REQUESTED_IP_LEN),
            Some([option::REQUESTED_IP, 4, 10, 0, 0, 50].as_slice())
        );
    }

    #[test]
    fn request_selecting_leaves_ciaddr_empty() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];
        let assigned = Ipv4Addr::new(10, 0, 0, 50);
        let server = Ipv4Addr::new(192, 168, 1, 1);

        // ACT
        let message = request(&mut buf, 0x1234, MAC, assigned, Mode::Selecting { server })
            .expect("message fits");

        // ASSERT
        assert_eq!(ciaddr(message), Ipv4Addr::UNSPECIFIED);
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
        assert_eq!(flags(message), FLAG_BROADCAST);
    }

    #[test]
    fn request_renew_omits_requested_ip_and_server_id() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];
        let assigned = Ipv4Addr::new(10, 0, 0, 50);

        // ACT
        let message = request(&mut buf, 0x1234, MAC, assigned, Mode::Renew).expect("message fits");

        // ASSERT
        let after_type = header::LEN + MESSAGE_TYPE_LEN;
        assert_eq!(message.len(), 250);
        assert_eq!(message.get(after_type), Some(&option::PARAM_REQUEST_LIST));
        assert!(!message.contains(&option::SERVER_ID));
    }

    #[test]
    fn request_rebind_omits_requested_ip_and_server_id() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];
        let assigned = Ipv4Addr::new(10, 0, 0, 50);

        // ACT
        let message = request(&mut buf, 0x1234, MAC, assigned, Mode::Rebind).expect("message fits");

        // ASSERT
        let after_type = header::LEN + MESSAGE_TYPE_LEN;
        assert_eq!(message.len(), 250);
        assert_eq!(message.get(after_type), Some(&option::PARAM_REQUEST_LIST));
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
        let Err(Failure::MessageTooLong {
            required: needed,
            capacity,
        }) = result
        else {
            panic!("expected MessageTooLong");
        };
        assert_eq!(needed, required);
        assert_eq!(capacity, required - 1);
    }

    #[test]
    fn request_rejects_short_buffer() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];
        let assigned = Ipv4Addr::new(10, 0, 0, 50);
        let required = request(&mut buf, 1, MAC, assigned, Mode::Renew)
            .expect("message fits")
            .len();

        // ACT
        let result = request(&mut buf[..required - 1], 1, MAC, assigned, Mode::Renew);

        // ASSERT
        let Err(Failure::MessageTooLong {
            required: needed,
            capacity,
        }) = result
        else {
            panic!("expected MessageTooLong");
        };
        assert_eq!(needed, required);
        assert_eq!(capacity, required - 1);
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
        let Err(Failure::MessageTooLong {
            required: needed,
            capacity,
        }) = result
        else {
            panic!("expected MessageTooLong");
        };
        assert_eq!(needed, required);
        assert_eq!(capacity, required - 1);
    }
}
