//! IPv6 upper-layer checksum support.

use core::net::Ipv6Addr;

use crate::checksum;

const PSEUDO_HEADER_TAIL_LEN: usize = 8;

/// Computes the upper-layer checksum over the RFC 8200 section 8.1 pseudo-header.
#[must_use = "callers need the checksum"]
pub fn sum(src: Ipv6Addr, dst: Ipv6Addr, next_header: u8, message: &[u8]) -> u16 {
    let tail = pseudo_tail(message.len(), next_header);
    let pseudo = checksum::accumulate(
        checksum::accumulate(checksum::accumulate(0, &src.octets()), &dst.octets()),
        &tail,
    );

    checksum::finish(checksum::accumulate(pseudo, message))
}

fn pseudo_tail(len: usize, next_header: u8) -> [u8; PSEUDO_HEADER_TAIL_LEN] {
    let length = u32::try_from(len).unwrap_or(u32::MAX).to_be_bytes();
    let mut tail = [0_u8; PSEUDO_HEADER_TAIL_LEN];
    let (length_field, tail_field) = tail.split_at_mut(4);
    length_field.copy_from_slice(&length);
    tail_field.copy_from_slice(&[0, 0, 0, next_header]);

    tail
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEXT_HEADER: u8 = 58;

    fn source() -> Ipv6Addr {
        Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1)
    }

    fn destination() -> Ipv6Addr {
        Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 2)
    }

    #[test]
    fn checksum_field_filled_by_sum_validates_to_zero() {
        // ARRANGE
        let mut message = [0x80_u8, 0, 0, 0, 0, 0, 0, 0];
        let checksum = sum(source(), destination(), NEXT_HEADER, &message);
        message[2..4].copy_from_slice(&checksum.to_be_bytes());

        // ACT
        let total = sum(source(), destination(), NEXT_HEADER, &message);

        // ASSERT
        assert_eq!(total, 0);
    }

    #[test]
    fn even_length_matches_pseudo_header_field_order() {
        // ARRANGE
        let message = [0x12_u8, 0x34, 0x56, 0x78];
        let source_octets = source().octets();
        let destination_octets = destination().octets();
        let tail = pseudo_tail(message.len(), NEXT_HEADER);

        // ACT
        let total = sum(source(), destination(), NEXT_HEADER, &message);
        let expected =
            checksum::sum_chunks(&[&source_octets, &destination_octets, &tail, &message]);

        // ASSERT
        assert_eq!(total, expected);
    }

    #[test]
    fn odd_length_pads_with_one_trailing_zero() {
        // ARRANGE
        let message = [0x80_u8, 0x00, 0x00];
        let mut padded = [0_u8; 44];
        padded[..16].copy_from_slice(&source().octets());
        padded[16..32].copy_from_slice(&destination().octets());
        padded[32..40].copy_from_slice(&pseudo_tail(message.len(), NEXT_HEADER));
        padded[40..43].copy_from_slice(&message);

        // ACT
        let total = sum(source(), destination(), NEXT_HEADER, &message);
        let expected = checksum::sum(&padded);

        // ASSERT
        assert_eq!(total, expected);
    }

    #[test]
    fn next_header_changes_the_result() {
        // ARRANGE
        let message = [0x12_u8, 0x34, 0x56, 0x78];

        // ACT
        let icmpv6 = sum(source(), destination(), 58, &message);
        let udp = sum(source(), destination(), 17, &message);

        // ASSERT
        assert_ne!(icmpv6, udp);
    }
}
