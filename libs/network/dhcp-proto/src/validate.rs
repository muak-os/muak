//! Validation of received `DHCPv4` responses.

use super::{MAGIC_COOKIE, ParsedOptions, field, message_type, options, xid};
use crate::error::{DhcpProtoError, Result};

/// Validates a received DHCP response and parses its options.
///
/// # Errors
///
/// Returns [`DhcpProtoError::ResponseTooShort`] when the message cannot hold the fixed header, the magic cookie and the
/// options, [`DhcpProtoError::XidMismatch`] when the transaction ID belongs to another exchange, [`DhcpProtoError::Nak`] when the
/// server refused the request, [`DhcpProtoError::MessageType`] when the message type is not `expected_type`, and
/// [`DhcpProtoError::MissingMessageType`] when the option is absent.
pub fn response(buf: &[u8], expected_xid: u32, expected_type: u8) -> Result<ParsedOptions<'_>> {
    let options_start = field::HEADER_LEN.saturating_add(MAGIC_COOKIE.len());
    if buf.len() < options_start {
        return Err(DhcpProtoError::ResponseTooShort(buf.len()));
    }

    if xid::extract(buf) != expected_xid {
        return Err(DhcpProtoError::XidMismatch);
    }

    let opts_bytes = buf
        .get(options_start..)
        .ok_or(DhcpProtoError::ResponseTooShort(buf.len()))?;
    let opts = options::parse(opts_bytes);

    match opts.message_type {
        Some(message_type::NAK) => Err(DhcpProtoError::Nak),
        Some(msg_type) if msg_type == expected_type => Ok(opts),
        Some(msg_type) => Err(DhcpProtoError::MessageType {
            expected: expected_type,
            got: msg_type,
        }),
        None => Err(DhcpProtoError::MissingMessageType),
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;

    use alloc::vec::Vec;

    use super::*;
    use crate::{header, option};

    const MAX_MESSAGE_LEN: usize = 1500;
    const MAC: [u8; 6] = [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];

    fn make_minimal_packet(xid: u32) -> Vec<u8> {
        let mut buf = [0_u8; MAX_MESSAGE_LEN];
        let header = header::build(&mut buf, xid, MAC).expect("header fits");

        header.to_vec()
    }

    #[test]
    fn response_too_short() {
        // ARRANGE
        let buf = [0_u8; 100];

        // ACT
        let result = response(&buf, 0, message_type::OFFER);

        // ASSERT
        assert!(matches!(result, Err(DhcpProtoError::ResponseTooShort(100))));
    }

    #[test]
    fn response_xid_mismatch() {
        // ARRANGE
        let mut pkt = make_minimal_packet(0x1111);
        pkt.extend([option::MESSAGE_TYPE, 1, message_type::OFFER, option::END]);

        // ACT
        let result = response(&pkt, 0x2222, message_type::OFFER);

        // ASSERT
        assert!(matches!(result, Err(DhcpProtoError::XidMismatch)));
    }

    #[test]
    fn response_wrong_message_type() {
        // ARRANGE
        let mut pkt = make_minimal_packet(0x1111);
        pkt.extend([option::MESSAGE_TYPE, 1, message_type::ACK, option::END]);

        // ACT
        let result = response(&pkt, 0x1111, message_type::OFFER);

        // ASSERT
        assert!(matches!(
            result,
            Err(DhcpProtoError::MessageType {
                expected: message_type::OFFER,
                got: message_type::ACK,
            })
        ));
    }

    #[test]
    fn response_missing_message_type() {
        // ARRANGE
        let mut pkt = make_minimal_packet(0x1111);
        pkt.push(option::END);

        // ACT
        let result = response(&pkt, 0x1111, message_type::OFFER);

        // ASSERT
        assert!(matches!(result, Err(DhcpProtoError::MissingMessageType)));
    }

    #[test]
    fn response_success() {
        // ARRANGE
        let mut pkt = make_minimal_packet(0xABCD);
        pkt.extend([option::MESSAGE_TYPE, 1, message_type::OFFER]);
        pkt.extend([option::LEASE_TIME, 4, 0, 0, 0x0E, 0x10]);
        pkt.push(option::END);

        // ACT
        let opts = response(&pkt, 0xABCD, message_type::OFFER).expect("should validate");

        // ASSERT
        assert_eq!(opts.message_type, Some(message_type::OFFER));
        assert_eq!(opts.lease_time, Some(3600));
    }

    #[test]
    fn response_nak_returns_nak_error() {
        // ARRANGE
        let mut pkt = make_minimal_packet(0x1234);
        pkt.extend([option::MESSAGE_TYPE, 1, message_type::NAK, option::END]);

        // ACT
        let result = response(&pkt, 0x1234, message_type::ACK);

        // ASSERT
        assert!(matches!(result, Err(DhcpProtoError::Nak)));
    }
}
