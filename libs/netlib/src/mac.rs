//! Deterministic MAC address generation.

use ring::digest;

/// Generates a deterministic locally-administered unicast MAC from an identifier.
#[must_use]
pub fn generate(id: &str) -> [u8; 6] {
    let result = digest::digest(&digest::SHA256, id.as_bytes());

    let mut mac = [0_u8; 6];
    for (dst, src) in mac.iter_mut().zip(result.as_ref()) {
        *dst = *src;
    }

    mac[0] = (mac[0] & 0xfe) | 0x02;

    mac
}

/// Formats a 6-byte MAC address as colon-separated lowercase hex.
#[must_use]
pub fn format(mac: &[u8; 6]) -> String {
    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_mac_sets_locally_administered_bit() {
        // ARRANGE / ACT
        let mac = generate("test-id");

        // ASSERT
        assert_eq!(mac[0] & 0x02, 0x02);
    }

    #[test]
    fn generate_mac_clears_multicast_bit() {
        // ARRANGE / ACT
        let mac = generate("test-id");

        // ASSERT
        assert_eq!(mac[0] & 0x01, 0x00);
    }

    #[test]
    fn generate_mac_is_deterministic() {
        // ACT
        let first_mac = generate("same-id");
        let second_mac = generate("same-id");

        // ASSERT
        assert_eq!(first_mac, second_mac);
    }

    #[test]
    fn generate_mac_differs_for_different_ids() {
        // ACT
        let first_mac = generate("alpha");
        let second_mac = generate("beta");

        // ASSERT
        assert_ne!(first_mac, second_mac);
    }

    #[test]
    fn generate_mac_empty_id_is_valid() {
        // ACT
        let mac = generate("");

        // ASSERT
        assert_eq!(mac[0] & 0x02, 0x02);
        assert_eq!(mac[0] & 0x01, 0x00);
    }

    #[test]
    fn format_mac_produces_colon_separated_hex() {
        // ARRANGE
        let mac = [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];

        // ACT / ASSERT
        assert_eq!(format(&mac), "aa:bb:cc:dd:ee:ff");
    }

    #[test]
    fn format_mac_zero_pads_single_digit_bytes() {
        // ARRANGE
        let mac = [0x02, 0x00, 0x0a, 0x00, 0x00, 0x01];

        // ACT / ASSERT
        assert_eq!(format(&mac), "02:00:0a:00:00:01");
    }

    #[test]
    fn format_mac_all_zeros() {
        // ARRANGE
        let mac = [0x00; 6];

        // ACT / ASSERT
        assert_eq!(format(&mac), "00:00:00:00:00:00");
    }

    #[test]
    fn format_mac_all_ff() {
        // ARRANGE
        let mac = [0xff; 6];

        // ACT / ASSERT
        assert_eq!(format(&mac), "ff:ff:ff:ff:ff:ff");
    }

    #[test]
    fn generate_then_format_roundtrip() {
        // ACT
        let mac = generate("roundtrip-test");
        let formatted = format(&mac);

        // ASSERT
        assert_eq!(formatted.len(), 17);
        assert_eq!(formatted.matches(':').count(), 5);
    }
}
