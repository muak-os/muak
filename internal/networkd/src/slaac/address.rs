use std::net::Ipv6Addr;

pub fn mac_to_eui64(mac: &[u8; 6]) -> [u8; 8] {
    [
        mac[0] ^ 0x02, // Flip universal/local bit
        mac[1],
        mac[2],
        0xff,
        0xfe,
        mac[3],
        mac[4],
        mac[5],
    ]
}

pub fn generate_slaac_address(prefix: Ipv6Addr, prefix_len: u8, mac: &[u8; 6]) -> Ipv6Addr {
    let prefix_bytes = prefix.octets();
    let eui64 = mac_to_eui64(mac);

    let prefix_bytes_count = (prefix_len / 8) as usize;
    let mut addr = [0u8; 16];

    addr[..prefix_bytes_count].copy_from_slice(&prefix_bytes[..prefix_bytes_count]);

    let interface_id_start = 8;
    addr[interface_id_start..16].copy_from_slice(&eui64);

    Ipv6Addr::from(addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mac_to_eui64() {
        // ARRANGE
        let mac = [0x00, 0x1a, 0x2b, 0x3c, 0x4d, 0x5e];

        // ACT
        let eui64 = mac_to_eui64(&mac);

        // ASSERT
        assert_eq!(eui64, [0x02, 0x1a, 0x2b, 0xff, 0xfe, 0x3c, 0x4d, 0x5e]);
    }

    #[test]
    fn test_generate_slaac_address() {
        // ARRANGE
        let prefix = "2001:db8:abcd:1234::".parse::<Ipv6Addr>().unwrap();
        let mac = [0x00, 0x1a, 0x2b, 0x3c, 0x4d, 0x5e];

        // ACT
        let addr = generate_slaac_address(prefix, 64, &mac);

        // ASSERT
        assert_eq!(
            addr,
            "2001:db8:abcd:1234:21a:2bff:fe3c:4d5e"
                .parse::<Ipv6Addr>()
                .unwrap()
        );
    }
}
