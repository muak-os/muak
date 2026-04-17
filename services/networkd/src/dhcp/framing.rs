//! IPv4 and UDP header construction and parsing for raw DHCP packets.

use std::net::Ipv4Addr;

use anyhow::{Result, bail};

/// Total length of an IPv4 header without options.
pub(crate) const IPV4_HEADER_LEN: usize = 20;
/// Total length of a UDP header.
pub(crate) const UDP_HEADER_LEN: usize = 8;
/// Combined IPv4+UDP header length prepended to the DHCP payload.
pub(crate) const L3L4_HEADER_LEN: usize = IPV4_HEADER_LEN + UDP_HEADER_LEN;

const IPV4_VERSION_IHL: u8 = 0x45;
const IPV4_TTL: u8 = 64;
const IPV4_PROTO_UDP: u8 = 17;
const IPV4_FLAG_DONT_FRAGMENT: u16 = 0x4000;

/// Wraps a UDP payload with IPv4+UDP headers and computed checksums.
pub(crate) fn wrap_ipv4_udp(
    payload: &[u8],
    src_ip: Ipv4Addr,
    dst_ip: Ipv4Addr,
    src_port: u16,
    dst_port: u16,
) -> Vec<u8> {
    let total_len = (L3L4_HEADER_LEN + payload.len()) as u16;
    let udp_len = (UDP_HEADER_LEN + payload.len()) as u16;

    let mut buf = Vec::with_capacity(total_len as usize);
    buf.push(IPV4_VERSION_IHL);
    buf.push(0);
    buf.extend_from_slice(&total_len.to_be_bytes());
    buf.extend_from_slice(&0u16.to_be_bytes());
    buf.extend_from_slice(&IPV4_FLAG_DONT_FRAGMENT.to_be_bytes());
    buf.push(IPV4_TTL);
    buf.push(IPV4_PROTO_UDP);
    buf.extend_from_slice(&0u16.to_be_bytes());
    buf.extend_from_slice(&src_ip.octets());
    buf.extend_from_slice(&dst_ip.octets());

    let ip_checksum = checksum_ones_complement(&buf[..IPV4_HEADER_LEN]);
    buf[10..12].copy_from_slice(&ip_checksum.to_be_bytes());

    buf.extend_from_slice(&src_port.to_be_bytes());
    buf.extend_from_slice(&dst_port.to_be_bytes());
    buf.extend_from_slice(&udp_len.to_be_bytes());
    buf.extend_from_slice(&0u16.to_be_bytes());
    buf.extend_from_slice(payload);

    let udp_checksum = udp_checksum(src_ip, dst_ip, &buf[IPV4_HEADER_LEN..]);
    let udp_checksum = if udp_checksum == 0 {
        0xFFFF
    } else {
        udp_checksum
    };
    buf[IPV4_HEADER_LEN + 6..IPV4_HEADER_LEN + 8].copy_from_slice(&udp_checksum.to_be_bytes());

    buf
}

/// Strips IPv4+UDP headers from a received packet, returning the UDP payload slice and src/dst ports.
pub(crate) fn unwrap_ipv4_udp(buf: &[u8]) -> Result<(&[u8], u16, u16)> {
    if buf.len() < L3L4_HEADER_LEN {
        bail!(
            "packet too short for IPv4+UDP headers ({} bytes)",
            buf.len()
        );
    }
    let ihl = (buf[0] & 0x0F) as usize * 4;
    if ihl < IPV4_HEADER_LEN || buf.len() < ihl + UDP_HEADER_LEN {
        bail!("invalid IPv4 header length {ihl}");
    }
    if buf[9] != IPV4_PROTO_UDP {
        bail!("non-UDP protocol {}", buf[9]);
    }
    let total_len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
    if total_len > buf.len() {
        bail!(
            "declared total length {total_len} exceeds buffer {}",
            buf.len()
        );
    }
    let udp = &buf[ihl..total_len];
    let src_port = u16::from_be_bytes([udp[0], udp[1]]);
    let dst_port = u16::from_be_bytes([udp[2], udp[3]]);
    let udp_len = u16::from_be_bytes([udp[4], udp[5]]) as usize;
    if udp_len < UDP_HEADER_LEN || udp_len > udp.len() {
        bail!("invalid UDP length {udp_len}");
    }
    Ok((&udp[UDP_HEADER_LEN..udp_len], src_port, dst_port))
}

/// Computes the 16-bit one's complement checksum over `data`.
fn checksum_ones_complement(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < data.len() {
        sum += u32::from(u16::from_be_bytes([data[i], data[i + 1]]));
        i += 2;
    }
    if i < data.len() {
        sum += u32::from(data[i]) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

/// Computes the UDP checksum including the IPv4 pseudo-header.
fn udp_checksum(src_ip: Ipv4Addr, dst_ip: Ipv4Addr, udp_segment: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let src = src_ip.octets();
    let dst = dst_ip.octets();
    sum += u32::from(u16::from_be_bytes([src[0], src[1]]));
    sum += u32::from(u16::from_be_bytes([src[2], src[3]]));
    sum += u32::from(u16::from_be_bytes([dst[0], dst[1]]));
    sum += u32::from(u16::from_be_bytes([dst[2], dst[3]]));
    sum += u32::from(IPV4_PROTO_UDP);
    sum += udp_segment.len() as u32;

    let mut i = 0;
    while i + 1 < udp_segment.len() {
        sum += u32::from(u16::from_be_bytes([udp_segment[i], udp_segment[i + 1]]));
        i += 2;
    }
    if i < udp_segment.len() {
        sum += u32::from(udp_segment[i]) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_then_unwrap_roundtrip() {
        // ARRANGE
        let payload = b"hello dhcp";
        let src = Ipv4Addr::new(0, 0, 0, 0);
        let dst = Ipv4Addr::new(255, 255, 255, 255);

        // ACT
        let frame = wrap_ipv4_udp(payload, src, dst, 68, 67);
        let (data, sp, dp) = unwrap_ipv4_udp(&frame).expect("unwrap ok");

        // ASSERT
        assert_eq!(data, payload);
        assert_eq!(sp, 68);
        assert_eq!(dp, 67);
    }

    #[test]
    fn wrap_sets_ip_total_length() {
        // ARRANGE
        let payload = vec![0u8; 300];

        // ACT
        let frame = wrap_ipv4_udp(&payload, Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST, 68, 67);

        // ASSERT
        let total = u16::from_be_bytes([frame[2], frame[3]]);
        assert_eq!(total as usize, IPV4_HEADER_LEN + UDP_HEADER_LEN + 300);
    }

    #[test]
    fn ip_header_checksum_validates() {
        // ARRANGE
        let payload = b"x";
        let frame = wrap_ipv4_udp(payload, Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST, 68, 67);

        // ACT
        let computed = checksum_ones_complement(&frame[..IPV4_HEADER_LEN]);

        // ASSERT
        assert_eq!(
            computed, 0,
            "IP header checksum must be zero when validated"
        );
    }

    #[test]
    fn udp_checksum_validates_with_pseudo_header() {
        // ARRANGE
        let src = Ipv4Addr::new(192, 168, 1, 10);
        let dst = Ipv4Addr::new(192, 168, 1, 1);
        let payload = b"check";
        let frame = wrap_ipv4_udp(payload, src, dst, 68, 67);

        // ACT
        let computed = udp_checksum(src, dst, &frame[IPV4_HEADER_LEN..]);

        // ASSERT
        assert_eq!(computed, 0);
    }

    #[test]
    fn unwrap_rejects_short_buffer() {
        // ARRANGE
        let buf = [0u8; 10];

        // ACT / ASSERT
        assert!(unwrap_ipv4_udp(&buf).is_err());
    }

    #[test]
    fn unwrap_rejects_non_udp_protocol() {
        // ARRANGE
        let mut frame = wrap_ipv4_udp(b"x", Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST, 68, 67);
        frame[9] = 6;

        // ACT / ASSERT
        assert!(unwrap_ipv4_udp(&frame).is_err());
    }
}
