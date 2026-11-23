//! Fixed `DHCPv4` header construction and field extraction (RFC 2131).

use core::net::Ipv4Addr;

use super::{BOOTREQUEST, FLAG_BROADCAST, MAGIC_COOKIE, field};
use crate::{Failure, Result};

const HTYPE_ETHERNET: u8 = 1;
const HLEN_ETHERNET: u8 = 6;

/// Size of the fixed header plus the magic cookie.
pub const LEN: usize = field::HEADER_LEN.saturating_add(MAGIC_COOKIE.len());

/// Writes the fixed header and magic cookie with an explicit `ciaddr` and `flags`.
///
/// `ciaddr` carries the address a RENEWING or REBINDING client already holds; it stays zero for
/// DISCOVER, SELECTING and all server messages.
///
/// # Errors
///
/// Returns [`Failure::MessageTooLong`] with the required byte count when `buf` is shorter than
/// [`LEN`].
pub fn build_with(
    buf: &mut [u8],
    xid: u32,
    mac: [u8; 6],
    ciaddr: Ipv4Addr,
    flags: u16,
) -> Result<&mut [u8]> {
    let Some(out) = buf.get_mut(..LEN) else {
        return Err(Failure::MessageTooLong {
            required: LEN,
            capacity: buf.len(),
        });
    };
    out.fill(0);
    set_byte(out, field::OP, BOOTREQUEST);
    set_byte(out, field::HTYPE, HTYPE_ETHERNET);
    set_byte(out, field::HLEN, HLEN_ETHERNET);
    set_byte(out, field::HOPS, 0);
    set_bytes(out, field::XID, &xid.to_be_bytes());
    set_bytes(out, field::CIADDR, &ciaddr.octets());
    set_bytes(out, field::FLAGS, &flags.to_be_bytes());
    set_bytes(out, field::CHADDR, &mac);
    set_bytes(out, field::HEADER_LEN, &MAGIC_COOKIE);

    Ok(out)
}

/// Writes the fixed header and magic cookie with `ciaddr` zero and the broadcast flag set.
///
/// # Errors
///
/// Returns [`Failure::MessageTooLong`] with the required byte count when `buf` is shorter than
/// [`LEN`].
pub fn build(buf: &mut [u8], xid: u32, mac: [u8; 6]) -> Result<&mut [u8]> {
    build_with(buf, xid, mac, Ipv4Addr::UNSPECIFIED, FLAG_BROADCAST)
}

/// Writes the fixed header and magic cookie with `ciaddr` zero and the broadcast flag cleared.
///
/// # Errors
///
/// Returns [`Failure::MessageTooLong`] with the required byte count when `buf` is shorter than
/// [`LEN`].
pub fn build_unicast(buf: &mut [u8], xid: u32, mac: [u8; 6]) -> Result<&mut [u8]> {
    build_with(buf, xid, mac, Ipv4Addr::UNSPECIFIED, 0)
}

/// Extracts `yiaddr` from a raw DHCP message, reading `0` for missing bytes.
#[must_use = "the address is the result of parsing"]
pub fn yiaddr(buf: &[u8]) -> Ipv4Addr {
    Ipv4Addr::from([
        byte_at(buf, field::YIADDR),
        byte_at(buf, field::YIADDR.saturating_add(1)),
        byte_at(buf, field::YIADDR.saturating_add(2)),
        byte_at(buf, field::YIADDR.saturating_add(3)),
    ])
}

fn set_byte(buf: &mut [u8], index: usize, value: u8) {
    if let Some(byte) = buf.get_mut(index) {
        *byte = value;
    }
}

fn set_bytes(buf: &mut [u8], start: usize, value: &[u8]) {
    let end = start.saturating_add(value.len());
    if let Some(slot) = buf.get_mut(start..end) {
        slot.copy_from_slice(value);
    }
}

fn byte_at(data: &[u8], index: usize) -> u8 {
    data.get(index).copied().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Buffer large enough for any message the builders produce.
    const MAX_MESSAGE_LEN: usize = 1500;
    const MAC: [u8; 6] = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];

    #[test]
    fn build_length() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];

        // ACT
        let header = build(&mut buf, 0x1234_5678, MAC).expect("header fits");

        // ASSERT
        assert_eq!(header.len(), LEN);
    }

    #[test]
    fn build_fields() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];

        // ACT
        let header = build(&mut buf, 0xDEAD_BEEF, MAC).expect("header fits");

        // ASSERT
        assert_eq!(byte_at(header, field::OP), BOOTREQUEST);
        assert_eq!(byte_at(header, field::HTYPE), HTYPE_ETHERNET);
        assert_eq!(byte_at(header, field::HLEN), HLEN_ETHERNET);
        assert_eq!(byte_at(header, field::HOPS), 0);
        assert_eq!(
            header.get(field::XID..field::XID + 4),
            Some(&0xDEAD_BEEF_u32.to_be_bytes()[..])
        );
        assert_eq!(header.get(field::CHADDR..field::CHADDR + 6), Some(&MAC[..]));
        assert_eq!(
            header.get(field::HEADER_LEN..field::HEADER_LEN + 4),
            Some(&MAGIC_COOKIE[..])
        );
    }

    #[test]
    fn build_with_writes_ciaddr() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];
        let assigned = Ipv4Addr::new(10, 0, 0, 50);

        // ACT
        let header = build_with(&mut buf, 1, MAC, assigned, 0).expect("header fits");

        // ASSERT
        assert_eq!(
            header.get(field::CIADDR..field::CIADDR + 4),
            Some(&assigned.octets()[..])
        );
    }

    #[test]
    fn build_with_writes_given_flags() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];

        // ACT
        let header = build_with(&mut buf, 1, MAC, Ipv4Addr::UNSPECIFIED, FLAG_BROADCAST)
            .expect("header fits");

        // ASSERT
        assert_eq!(
            header.get(field::FLAGS..field::FLAGS + 2),
            Some(&FLAG_BROADCAST.to_be_bytes()[..])
        );
    }

    #[test]
    fn build_sets_broadcast_flag() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];

        // ACT
        let header = build(&mut buf, 1, [0; 6]).expect("header fits");

        // ASSERT
        assert_eq!(
            u16::from_be_bytes([
                byte_at(header, field::FLAGS),
                byte_at(header, field::FLAGS.saturating_add(1)),
            ]),
            FLAG_BROADCAST
        );
    }

    #[test]
    fn build_unicast_clears_broadcast_flag() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];

        // ACT
        let header = build_unicast(&mut buf, 1, [0; 6]).expect("header fits");

        // ASSERT
        assert_eq!(
            u16::from_be_bytes([
                byte_at(header, field::FLAGS),
                byte_at(header, field::FLAGS.saturating_add(1)),
            ]),
            0
        );
    }

    #[test]
    fn build_clears_reused_buffer() {
        // ARRANGE
        let mut buf = [0xAA_u8; 300];
        let secs = 8..10;
        let ciaddr = 12..16;
        let yiaddr = 16..20;

        // ACT
        let header = build(&mut buf, 0x1234, MAC).expect("header fits");

        // ASSERT
        assert!(
            header
                .get(secs)
                .is_some_and(|slot| slot.iter().all(|byte| *byte == 0))
        );
        assert!(
            header
                .get(ciaddr)
                .is_some_and(|slot| slot.iter().all(|byte| *byte == 0))
        );
        assert!(
            header
                .get(yiaddr)
                .is_some_and(|slot| slot.iter().all(|byte| *byte == 0))
        );
    }

    #[test]
    fn build_rejects_short_buffer() {
        // ARRANGE
        let mut buf = [0_u8; LEN - 1];

        // ACT
        let result = build(&mut buf, 1, MAC);

        // ASSERT
        let Err(Failure::MessageTooLong { required, capacity }) = result else {
            panic!("expected MessageTooLong");
        };
        assert_eq!(required, LEN);
        assert_eq!(capacity, LEN - 1);
    }

    #[test]
    fn yiaddr_extraction() {
        // ARRANGE
        let mut buf = [0_u8; MAX_MESSAGE_LEN];
        let header = build(&mut buf, 1, MAC).expect("header fits");
        let assigned = Ipv4Addr::new(10, 0, 0, 42);
        let slot = header
            .get_mut(field::YIADDR..field::YIADDR + 4)
            .expect("header has a yiaddr field");
        slot.copy_from_slice(&assigned.octets());

        // ACT
        let result = yiaddr(header);

        // ASSERT
        assert_eq!(result, assigned);
    }
}
