//! IPv4 header construction and parsing, as defined by RFC 791.

use core::net::Ipv4Addr;

use crate::{Failure, Result, checksum};

/// Length of an IPv4 header without options.
pub(crate) const IPV4_HEADER_LEN: usize = 20;

const IPV4_VERSION_IHL: u8 = 0x45;
const IPV4_TTL: u8 = 64;
const IPV4_FLAG_DONT_FRAGMENT: u16 = 0x4000;

/// IPv4 header fields needed to locate and identify the payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// Header length in bytes, options included.
    pub ihl: usize,
    /// Payload protocol number.
    pub protocol: u8,
    /// Total datagram length declared by the header, header included.
    pub total_len: usize,
    /// Source address.
    pub src: Ipv4Addr,
    /// Destination address.
    pub dst: Ipv4Addr,
}

/// Writes a 20 byte IPv4 header at the start of `buf` and returns it.
///
/// # Errors
///
/// Returns [`Failure::TooSmall`] when `buf` cannot hold a header and
/// [`Failure::LengthOverflow`] when the declared total length exceeds [`u16::MAX`].
pub fn build(
    src: Ipv4Addr,
    dst: Ipv4Addr,
    protocol: u8,
    payload_len: usize,
    buf: &mut [u8],
) -> Result<&mut [u8]> {
    let head = buf
        .get_mut(..IPV4_HEADER_LEN)
        .ok_or(Failure::TooSmall(IPV4_HEADER_LEN))?;
    let total_len = IPV4_HEADER_LEN.saturating_add(payload_len);
    let total_len =
        u16::try_from(total_len).map_err(|_error| Failure::LengthOverflow(total_len))?;

    super::slot(head, 0..1)
        .ok_or(Failure::TooSmall(IPV4_HEADER_LEN))?
        .copy_from_slice(&[IPV4_VERSION_IHL]);
    super::slot(head, 1..2)
        .ok_or(Failure::TooSmall(IPV4_HEADER_LEN))?
        .copy_from_slice(&[0]);
    super::slot(head, 2..4)
        .ok_or(Failure::TooSmall(IPV4_HEADER_LEN))?
        .copy_from_slice(&total_len.to_be_bytes());
    super::slot(head, 4..6)
        .ok_or(Failure::TooSmall(IPV4_HEADER_LEN))?
        .copy_from_slice(&0_u16.to_be_bytes());
    super::slot(head, 6..8)
        .ok_or(Failure::TooSmall(IPV4_HEADER_LEN))?
        .copy_from_slice(&IPV4_FLAG_DONT_FRAGMENT.to_be_bytes());
    super::slot(head, 8..9)
        .ok_or(Failure::TooSmall(IPV4_HEADER_LEN))?
        .copy_from_slice(&[IPV4_TTL]);
    super::slot(head, 9..10)
        .ok_or(Failure::TooSmall(IPV4_HEADER_LEN))?
        .copy_from_slice(&[protocol]);
    super::slot(head, 10..12)
        .ok_or(Failure::TooSmall(IPV4_HEADER_LEN))?
        .copy_from_slice(&0_u16.to_be_bytes());
    super::slot(head, 12..16)
        .ok_or(Failure::TooSmall(IPV4_HEADER_LEN))?
        .copy_from_slice(&src.octets());
    super::slot(head, 16..20)
        .ok_or(Failure::TooSmall(IPV4_HEADER_LEN))?
        .copy_from_slice(&dst.octets());

    let ip_checksum = checksum::sum(head);
    super::slot(head, 10..12)
        .ok_or(Failure::TooSmall(IPV4_HEADER_LEN))?
        .copy_from_slice(&ip_checksum.to_be_bytes());

    Ok(head)
}

/// Parses the IPv4 header at the start of `buf`.
///
/// # Errors
///
/// Returns [`Failure::Truncated`] when `buf` is shorter than the minimum header or than the
/// declared total length, and [`Failure::BadIhl`] when the IHL field is below the minimum header
/// length or the IHL-derived extent does not fit the buffer. A declared total length below the
/// header length is reported as [`Failure::Truncated`] as well.
pub fn parse(buf: &[u8]) -> Result<Header> {
    if buf.len() < IPV4_HEADER_LEN {
        return Err(Failure::Truncated(buf.len(), IPV4_HEADER_LEN));
    }
    let ihl = usize::from(byte_at(buf, 0) & 0x0F).saturating_mul(4);
    if ihl < IPV4_HEADER_LEN || buf.len() < ihl {
        return Err(Failure::BadIhl(ihl));
    }
    let total_len = usize::from(u16::from_be_bytes([byte_at(buf, 2), byte_at(buf, 3)]));
    if total_len < ihl {
        return Err(Failure::Truncated(total_len, ihl));
    }
    if total_len > buf.len() {
        return Err(Failure::Truncated(buf.len(), total_len));
    }

    Ok(Header {
        ihl,
        protocol: byte_at(buf, 9),
        total_len,
        src: read_addr(buf, 12),
        dst: read_addr(buf, 16),
    })
}

fn read_addr(data: &[u8], offset: usize) -> Ipv4Addr {
    Ipv4Addr::new(
        byte_at(data, offset),
        byte_at(data, offset.saturating_add(1)),
        byte_at(data, offset.saturating_add(2)),
        byte_at(data, offset.saturating_add(3)),
    )
}

fn byte_at(data: &[u8], index: usize) -> u8 {
    data.get(index).copied().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v4::{Addr, udp};

    #[test]
    fn parse_returns_protocol_length_and_addresses() {
        // ARRANGE
        let src = Addr {
            ip: Ipv4Addr::new(10, 0, 0, 2),
            port: 68,
        };
        let dst = Addr {
            ip: Ipv4Addr::BROADCAST,
            port: 67,
        };
        let payload = b"payload";
        let mut buf = [0_u8; 64];
        let frame = udp::wrap(payload, src, dst, &mut buf).expect("wrap writes the frame");

        // ACT
        let header = parse(frame).expect("parse reads a wrapped frame");

        // ASSERT
        assert_eq!(header.ihl, IPV4_HEADER_LEN);
        assert_eq!(header.protocol, 17);
        assert_eq!(
            header.total_len,
            IPV4_HEADER_LEN
                .saturating_add(udp::UDP_HEADER_LEN)
                .saturating_add(payload.len())
        );
        assert_eq!(header.src, src.ip);
        assert_eq!(header.dst, dst.ip);
    }

    #[test]
    fn parse_accepts_non_udp_header() {
        // ARRANGE
        let src = Ipv4Addr::new(10, 0, 0, 1);
        let dst = Ipv4Addr::new(10, 0, 0, 2);
        let mut buf = [0_u8; IPV4_HEADER_LEN];
        let written = build(src, dst, 1, 0, &mut buf).expect("build writes a header");

        // ACT
        let header = parse(written).expect("parse accepts a non-UDP datagram with no payload");

        // ASSERT
        assert_eq!(header.protocol, 1);
        assert_eq!(header.total_len, IPV4_HEADER_LEN);
    }

    #[test]
    fn parse_rejects_total_length_below_ihl() {
        // ARRANGE
        let src = Ipv4Addr::new(10, 0, 0, 1);
        let dst = Ipv4Addr::new(10, 0, 0, 2);
        let mut buf = [0_u8; IPV4_HEADER_LEN];
        build(src, dst, 17, 0, &mut buf).expect("build writes a header");
        let total_length = buf.get_mut(2..4).expect("header has a total length field");
        total_length.copy_from_slice(&10_u16.to_be_bytes());

        // ACT
        let result = parse(&buf);

        // ASSERT
        assert_eq!(result, Err(Failure::Truncated(10, IPV4_HEADER_LEN)));
    }
}
