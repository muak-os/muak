//! UDP segment framing over IPv4, as defined by RFC 768.

use core::net::Ipv4Addr;

use super::Addr;
use crate::checksum;
use crate::error::{IpError, Result};
use crate::v4::header;

/// Length of a UDP header.
pub(crate) const UDP_HEADER_LEN: usize = 8;

const PROTOCOL_UDP: u8 = 17;

/// Writes an IPv4 header, a UDP header and `payload` into `buf`.
///
/// # Errors
///
/// Returns [`IpError::TooSmall`] when `buf` cannot hold both headers and the payload, and [`IpError::LengthOverflow`]
/// when either length field overflows [`u16::MAX`].
pub fn wrap<'a>(payload: &[u8], src: Addr, dst: Addr, buf: &'a mut [u8]) -> Result<&'a mut [u8]> {
    let udp_len = UDP_HEADER_LEN.saturating_add(payload.len());
    let frame_len = header::IPV4_HEADER_LEN.saturating_add(udp_len);
    let frame = buf
        .get_mut(..frame_len)
        .ok_or(IpError::TooSmall(frame_len))?;
    let (ip, segment) = frame.split_at_mut(header::IPV4_HEADER_LEN);

    write_segment(payload, src, dst, segment)?;
    let _ip = header::build(src.ip, dst.ip, PROTOCOL_UDP, udp_len, ip)?;

    Ok(frame)
}

/// Parses the IPv4 and UDP headers of `buf`.
///
/// # Errors
///
/// Returns whatever [`header::parse`] returns for a malformed IPv4 header, [`IpError::NotUdp`] when the protocol is not
/// UDP, and [`IpError::Truncated`] when the datagram is too short to hold a UDP header or when a length field is
/// inconsistent: a total length below the IHL, a UDP length below the 8 byte UDP header, or a UDP length past the end
/// of the segment.
pub fn unwrap(buf: &[u8]) -> Result<(Addr, Addr, &[u8])> {
    let header = header::parse(buf)?;
    if header.protocol != PROTOCOL_UDP {
        return Err(IpError::NotUdp(header.protocol));
    }
    let segment = buf
        .get(header.ihl..header.total_len)
        .ok_or(IpError::Truncated(header.total_len, header.ihl))?;
    if segment.len() < UDP_HEADER_LEN {
        return Err(IpError::Truncated(segment.len(), UDP_HEADER_LEN));
    }
    let udp_len = usize::from(read_u16(segment, 4)?);
    if udp_len < UDP_HEADER_LEN {
        return Err(IpError::Truncated(udp_len, UDP_HEADER_LEN));
    }
    let payload = segment
        .get(UDP_HEADER_LEN..udp_len)
        .ok_or(IpError::Truncated(segment.len(), udp_len))?;

    Ok((
        Addr {
            ip: header.src,
            port: read_u16(segment, 0)?,
        },
        Addr {
            ip: header.dst,
            port: read_u16(segment, 2)?,
        },
        payload,
    ))
}

fn write_segment(payload: &[u8], src: Addr, dst: Addr, segment: &mut [u8]) -> Result<()> {
    let udp_len =
        u16::try_from(segment.len()).map_err(|_error| IpError::LengthOverflow(segment.len()))?;

    super::slot(segment, 0..2)
        .ok_or(IpError::TooSmall(UDP_HEADER_LEN))?
        .copy_from_slice(&src.port.to_be_bytes());
    super::slot(segment, 2..4)
        .ok_or(IpError::TooSmall(UDP_HEADER_LEN))?
        .copy_from_slice(&dst.port.to_be_bytes());
    super::slot(segment, 4..6)
        .ok_or(IpError::TooSmall(UDP_HEADER_LEN))?
        .copy_from_slice(&udp_len.to_be_bytes());
    super::slot(segment, 6..8)
        .ok_or(IpError::TooSmall(UDP_HEADER_LEN))?
        .copy_from_slice(&0_u16.to_be_bytes());
    let payload_end = segment.len();
    super::slot(segment, UDP_HEADER_LEN..payload_end)
        .ok_or(IpError::TooSmall(UDP_HEADER_LEN))?
        .copy_from_slice(payload);

    let udp_checksum = segment_checksum(src.ip, dst.ip, udp_len, segment);
    let udp_checksum = if udp_checksum == 0 {
        0xFFFF
    } else {
        udp_checksum
    };
    super::slot(segment, 6..8)
        .ok_or(IpError::TooSmall(UDP_HEADER_LEN))?
        .copy_from_slice(&udp_checksum.to_be_bytes());

    Ok(())
}

fn segment_checksum(src: Ipv4Addr, dst: Ipv4Addr, udp_len: u16, segment: &[u8]) -> u16 {
    let mut sum = checksum::accumulate(0, &src.octets());
    sum = checksum::accumulate(sum, &dst.octets());
    sum = sum.saturating_add(u32::from(PROTOCOL_UDP));
    sum = sum.saturating_add(u32::from(udp_len));

    checksum::finish(checksum::accumulate(sum, segment))
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16> {
    let word = data
        .get(offset..)
        .and_then(<[u8]>::first_chunk::<2>)
        .ok_or(IpError::Truncated(data.len(), offset.saturating_add(2)))?;

    Ok(u16::from_be_bytes(*word))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoints() -> (Addr, Addr) {
        (
            Addr {
                ip: Ipv4Addr::new(192, 168, 1, 10),
                port: 68,
            },
            Addr {
                ip: Ipv4Addr::new(192, 168, 1, 1),
                port: 67,
            },
        )
    }

    #[test]
    fn wrap_then_unwrap_roundtrip() {
        // ARRANGE
        let payload = b"hello dhcp";
        let (src, dst) = endpoints();
        let mut buf = [0_u8; 64];
        let frame = wrap(payload, src, dst, &mut buf).expect("wrap accepts a large enough buffer");

        // ACT
        let (from, to, data) = unwrap(frame).expect("unwrap reads a wrapped frame");

        // ASSERT
        assert_eq!(data, payload.as_slice());
        assert_eq!(from, src);
        assert_eq!(to, dst);
    }

    #[test]
    fn wrap_sets_ip_total_length() {
        // ARRANGE
        let payload = [0_u8; 300];
        let (src, dst) = endpoints();
        let mut buf = [0_u8; 512];
        let frame = wrap(&payload, src, dst, &mut buf).expect("wrap accepts a large enough buffer");

        // ACT
        let header = header::parse(frame).expect("parse reads a wrapped frame");

        // ASSERT
        assert_eq!(
            header.total_len,
            header::IPV4_HEADER_LEN + UDP_HEADER_LEN + payload.len()
        );
    }

    #[test]
    fn ip_header_checksum_validates() {
        // ARRANGE
        let (src, dst) = endpoints();
        let mut buf = [0_u8; 64];
        let frame = wrap(b"x", src, dst, &mut buf).expect("wrap accepts a large enough buffer");
        let ip = frame
            .get(..header::IPV4_HEADER_LEN)
            .expect("frame starts with an IPv4 header");

        // ACT
        let computed = checksum::sum(ip);

        // ASSERT
        assert_eq!(computed, 0, "IPv4 header checksum must validate to zero");
    }

    #[test]
    fn udp_checksum_validates_with_pseudo_header() {
        // ARRANGE
        let payload = b"check";
        let (src, dst) = endpoints();
        let mut buf = [0_u8; 64];
        let frame = wrap(payload, src, dst, &mut buf).expect("wrap accepts a large enough buffer");
        let segment = frame
            .get(header::IPV4_HEADER_LEN..)
            .expect("frame continues with a UDP segment");
        let udp_len = u16::try_from(segment.len()).expect("segment fits a UDP length field");

        // ACT
        let computed = segment_checksum(src.ip, dst.ip, udp_len, segment);

        // ASSERT
        assert_eq!(computed, 0, "UDP checksum must validate to zero");
    }

    #[test]
    fn wrap_rejects_too_small_buffer() {
        // ARRANGE
        let (src, dst) = endpoints();
        let mut buf = [0_u8; 10];

        // ACT
        let result = wrap(b"payload", src, dst, &mut buf);

        // ASSERT
        assert_eq!(
            result,
            Err(IpError::TooSmall(
                header::IPV4_HEADER_LEN + UDP_HEADER_LEN + "payload".len()
            ))
        );
    }

    #[test]
    fn unwrap_rejects_short_buffer() {
        // ARRANGE
        let buf = [0_u8; 10];

        // ACT
        let result = unwrap(&buf);

        // ASSERT
        assert_eq!(
            result,
            Err(IpError::Truncated(buf.len(), header::IPV4_HEADER_LEN))
        );
    }

    #[test]
    fn unwrap_rejects_non_udp_protocol() {
        // ARRANGE
        let (src, dst) = endpoints();
        let mut buf = [0_u8; 64];
        let frame = wrap(b"x", src, dst, &mut buf).expect("wrap accepts a large enough buffer");
        let protocol = frame.get_mut(9).expect("frame has a protocol field");
        *protocol = 6;

        // ACT
        let result = unwrap(frame);

        // ASSERT
        assert_eq!(result, Err(IpError::NotUdp(6)));
    }

    #[test]
    fn unwrap_rejects_bad_ihl() {
        // ARRANGE
        let (src, dst) = endpoints();
        let mut buf = [0_u8; 64];
        let frame = wrap(b"x", src, dst, &mut buf).expect("wrap accepts a large enough buffer");
        let version_ihl = frame.get_mut(0).expect("frame has a version and IHL byte");
        *version_ihl = 0x44;

        // ACT
        let result = unwrap(frame);

        // ASSERT
        assert_eq!(result, Err(IpError::BadIhl(16)));
    }

    #[test]
    fn unwrap_rejects_declared_udp_length_outside_segment() {
        // ARRANGE
        let (src, dst) = endpoints();
        let mut buf = [0_u8; 64];
        let frame = wrap(b"x", src, dst, &mut buf).expect("wrap accepts a large enough buffer");
        let segment_len = frame.len().saturating_sub(header::IPV4_HEADER_LEN);
        let declared = u16::try_from(segment_len.saturating_add(1)).expect("segment fits a u16");
        let length_start = header::IPV4_HEADER_LEN.saturating_add(4);
        let length_field = frame
            .get_mut(length_start..length_start.saturating_add(2))
            .expect("frame has a UDP length field");
        length_field.copy_from_slice(&declared.to_be_bytes());

        // ACT
        let result = unwrap(frame);

        // ASSERT
        assert_eq!(
            result,
            Err(IpError::Truncated(segment_len, usize::from(declared)))
        );
    }

    #[test]
    fn unwrap_rejects_udp_length_below_header() {
        // ARRANGE
        let (src, dst) = endpoints();
        let mut buf = [0_u8; 64];
        let frame = wrap(b"x", src, dst, &mut buf).expect("wrap accepts a large enough buffer");
        let length_start = header::IPV4_HEADER_LEN.saturating_add(4);
        let length_field = frame
            .get_mut(length_start..length_start.saturating_add(2))
            .expect("frame has a UDP length field");
        length_field.copy_from_slice(&4_u16.to_be_bytes());

        // ACT
        let result = unwrap(frame);

        // ASSERT
        assert_eq!(result, Err(IpError::Truncated(4, UDP_HEADER_LEN)));
    }

    #[test]
    fn unwrap_rejects_total_length_beyond_buffer() {
        // ARRANGE
        let (src, dst) = endpoints();
        let mut buf = [0_u8; 64];
        let frame = wrap(b"x", src, dst, &mut buf).expect("wrap accepts a large enough buffer");
        let frame_len = frame.len();
        let declared = u16::try_from(frame_len.saturating_add(1)).expect("frame fits a u16");
        let total_len = frame.get_mut(2..4).expect("frame has a total length field");
        total_len.copy_from_slice(&declared.to_be_bytes());

        // ACT
        let result = unwrap(frame);

        // ASSERT
        assert_eq!(
            result,
            Err(IpError::Truncated(frame_len, usize::from(declared)))
        );
    }
}
