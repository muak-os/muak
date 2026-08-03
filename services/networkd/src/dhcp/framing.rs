//! IPv4 and UDP header construction and parsing for raw DHCP packets.

use core::net::Ipv4Addr;

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
    let total_len =
        u16::try_from(L3L4_HEADER_LEN.saturating_add(payload.len())).unwrap_or(u16::MAX);
    let udp_len = u16::try_from(UDP_HEADER_LEN.saturating_add(payload.len())).unwrap_or(u16::MAX);

    let mut ip_header = [0_u8; IPV4_HEADER_LEN];
    ip_header[0] = IPV4_VERSION_IHL;
    ip_header[1] = 0;
    ip_header[2..4].copy_from_slice(&total_len.to_be_bytes());
    ip_header[4..6].copy_from_slice(&0_u16.to_be_bytes());
    ip_header[6..8].copy_from_slice(&IPV4_FLAG_DONT_FRAGMENT.to_be_bytes());
    ip_header[8] = IPV4_TTL;
    ip_header[9] = IPV4_PROTO_UDP;
    ip_header[10..12].copy_from_slice(&0_u16.to_be_bytes());
    ip_header[12..16].copy_from_slice(&src_ip.octets());
    ip_header[16..20].copy_from_slice(&dst_ip.octets());

    let ip_checksum = checksum_ones_complement(&ip_header);
    ip_header[10..12].copy_from_slice(&ip_checksum.to_be_bytes());

    let mut buf = Vec::with_capacity(usize::from(total_len));
    buf.extend_from_slice(&ip_header);
    buf.extend_from_slice(&src_port.to_be_bytes());
    buf.extend_from_slice(&dst_port.to_be_bytes());
    buf.extend_from_slice(&udp_len.to_be_bytes());
    buf.extend_from_slice(&0_u16.to_be_bytes());
    buf.extend_from_slice(payload);

    let udp_checksum = udp_checksum(
        src_ip,
        dst_ip,
        buf.get(IPV4_HEADER_LEN..).unwrap_or_default(),
    );
    let udp_checksum = if udp_checksum == 0 {
        0xFFFF
    } else {
        udp_checksum
    };
    if let Some(slot) = buf.get_mut(IPV4_HEADER_LEN + 6..IPV4_HEADER_LEN + 8) {
        slot.copy_from_slice(&udp_checksum.to_be_bytes());
    }

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
    let ihl = usize::from(byte_at(buf, 0) & 0x0F).saturating_mul(4);
    if ihl < IPV4_HEADER_LEN || buf.len() < ihl.saturating_add(UDP_HEADER_LEN) {
        bail!("invalid IPv4 header length {ihl}");
    }
    if byte_at(buf, 9) != IPV4_PROTO_UDP {
        bail!("non-UDP protocol {}", byte_at(buf, 9));
    }
    let total_len = usize::from(u16::from_be_bytes([byte_at(buf, 2), byte_at(buf, 3)]));
    if total_len > buf.len() {
        bail!(
            "declared total length {total_len} exceeds buffer {}",
            buf.len()
        );
    }
    let udp = buf.get(ihl..total_len).unwrap_or_default();
    let src_port = read_u16(udp, 0);
    let dst_port = read_u16(udp, 2);
    let udp_len = usize::from(read_u16(udp, 4));
    if udp_len < UDP_HEADER_LEN || udp_len > udp.len() {
        bail!("invalid UDP length {udp_len}");
    }

    Ok((
        udp.get(UDP_HEADER_LEN..udp_len).unwrap_or_default(),
        src_port,
        dst_port,
    ))
}

/// Computes the UDP checksum including the IPv4 pseudo-header.
fn udp_checksum(src_ip: Ipv4Addr, dst_ip: Ipv4Addr, udp_segment: &[u8]) -> u16 {
    let src = src_ip.octets();
    let dst = dst_ip.octets();
    let mut sum = 0_u32;
    sum = sum.saturating_add(u32::from(u16::from_be_bytes([src[0], src[1]])));
    sum = sum.saturating_add(u32::from(u16::from_be_bytes([src[2], src[3]])));
    sum = sum.saturating_add(u32::from(u16::from_be_bytes([dst[0], dst[1]])));
    sum = sum.saturating_add(u32::from(u16::from_be_bytes([dst[2], dst[3]])));
    sum = sum.saturating_add(u32::from(IPV4_PROTO_UDP));
    sum = sum.saturating_add(u32::try_from(udp_segment.len()).unwrap_or(0));

    finish_checksum(checksum_words(sum, udp_segment))
}

/// Computes the 16-bit one's complement checksum over `data`.
fn checksum_ones_complement(data: &[u8]) -> u16 {
    finish_checksum(checksum_words(0, data))
}

/// Accumulates big-endian 16-bit words from `data` into `sum`.
fn checksum_words(mut sum: u32, data: &[u8]) -> u32 {
    let mut bytes = data.iter().copied();
    while let Some(high) = bytes.next() {
        let low = bytes.next().unwrap_or(0);
        sum = sum.saturating_add(u32::from(u16::from_be_bytes([high, low])));
    }

    sum
}

/// Folds `sum` into the final one's complement checksum.
fn finish_checksum(mut sum: u32) -> u16 {
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF).saturating_add(sum >> 16);
    }

    !u16::try_from(sum).unwrap_or(0)
}

/// Reads a big-endian `u16` starting at `offset`.
fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([
        byte_at(data, offset),
        byte_at(data, offset.saturating_add(1)),
    ])
}

/// Reads a byte at `index`, returning `0` when out of bounds.
fn byte_at(data: &[u8], index: usize) -> u8 {
    data.get(index).copied().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_then_unwrap_roundtrip() {
        // ARRANGE
        let payload = b"hello dhcp";
        let src = Ipv4Addr::UNSPECIFIED;
        let dst = Ipv4Addr::BROADCAST;

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
        let payload = vec![0_u8; 300];

        // ACT
        let frame = wrap_ipv4_udp(&payload, Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST, 68, 67);

        // ASSERT
        let total = u16::from_be_bytes([byte_at(&frame, 2), byte_at(&frame, 3)]);
        assert_eq!(usize::from(total), IPV4_HEADER_LEN + UDP_HEADER_LEN + 300);
    }

    #[test]
    fn ip_header_checksum_validates() {
        // ARRANGE
        let payload = b"x";
        let frame = wrap_ipv4_udp(payload, Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST, 68, 67);

        // ACT
        let computed = checksum_ones_complement(frame.get(..IPV4_HEADER_LEN).unwrap_or_default());

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
        let computed = udp_checksum(src, dst, frame.get(IPV4_HEADER_LEN..).unwrap_or_default());

        // ASSERT
        assert_eq!(computed, 0);
    }

    #[test]
    fn unwrap_rejects_short_buffer() {
        // ARRANGE
        let buf = [0_u8; 10];

        // ACT / ASSERT
        unwrap_ipv4_udp(&buf).unwrap_err();
    }

    #[test]
    fn unwrap_rejects_non_udp_protocol() {
        // ARRANGE
        let mut frame = wrap_ipv4_udp(b"x", Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST, 68, 67);
        if let Some(protocol) = frame.get_mut(9) {
            *protocol = 6;
        }

        // ACT / ASSERT
        unwrap_ipv4_udp(&frame).unwrap_err();
    }
}
