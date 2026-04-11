//! SLAAC IPv6 address generation from a MAC address and advertised prefix.

use std::net::Ipv6Addr;

/// Converts a 6-byte MAC address into an 8-byte EUI-64 interface identifier.
pub fn mac_to_eui64(mac: &[u8; 6]) -> [u8; 8] {
    [
        mac[0] ^ 0x02,
        mac[1],
        mac[2],
        0xff,
        0xfe,
        mac[3],
        mac[4],
        mac[5],
    ]
}

/// Generates a SLAAC address by combining a network prefix with an EUI-64 host identifier.
pub fn generate(prefix: Ipv6Addr, prefix_len: u8, mac: &[u8; 6]) -> Option<Ipv6Addr> {
    if prefix_len > 64 || !prefix_len.is_multiple_of(8) {
        return None;
    }

    let prefix_bytes = prefix.octets();
    let eui64 = mac_to_eui64(mac);

    let prefix_bytes_count = (prefix_len / 8) as usize;
    let mut addr = [0u8; 16];

    addr[..prefix_bytes_count].copy_from_slice(&prefix_bytes[..prefix_bytes_count]);
    addr[8..16].copy_from_slice(&eui64);

    Some(Ipv6Addr::from(addr))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eui64_flips_universal_local_bit() {
        // ARRANGE
        let mac = [0x00, 0x1a, 0x2b, 0x3c, 0x4d, 0x5e];

        // ACT
        let eui64 = mac_to_eui64(&mac);

        // ASSERT
        assert_eq!(eui64, [0x02, 0x1a, 0x2b, 0xff, 0xfe, 0x3c, 0x4d, 0x5e]);
    }

    #[test]
    fn slaac_address_from_prefix_and_mac() {
        // ARRANGE
        let prefix = "2001:db8:abcd:1234::"
            .parse::<Ipv6Addr>()
            .expect("valid prefix");
        let mac = [0x00, 0x1a, 0x2b, 0x3c, 0x4d, 0x5e];

        // ACT
        let addr = generate(prefix, 64, &mac);

        // ASSERT
        assert_eq!(
            addr,
            Some(
                "2001:db8:abcd:1234:21a:2bff:fe3c:4d5e"
                    .parse::<Ipv6Addr>()
                    .expect("valid address")
            )
        );
    }

    #[test]
    fn slaac_address_rejects_prefix_len_above_64() {
        // ARRANGE
        let prefix = "2001:db8::".parse::<Ipv6Addr>().expect("valid prefix");
        let mac = [0x00, 0x1a, 0x2b, 0x3c, 0x4d, 0x5e];

        // ACT / ASSERT
        assert!(generate(prefix, 65, &mac).is_none());
        assert!(generate(prefix, 128, &mac).is_none());
    }

    #[test]
    fn slaac_address_rejects_non_byte_aligned_prefix_len() {
        // ARRANGE
        let prefix = "2001:db8::".parse::<Ipv6Addr>().expect("valid prefix");
        let mac = [0x00, 0x1a, 0x2b, 0x3c, 0x4d, 0x5e];

        // ACT / ASSERT
        assert!(generate(prefix, 63, &mac).is_none());
        assert!(generate(prefix, 1, &mac).is_none());
    }
}
