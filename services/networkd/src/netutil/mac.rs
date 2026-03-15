use ring::digest;

pub fn generate_mac_address(vm_id: &str) -> [u8; 6] {
    let result = digest::digest(&digest::SHA256, vm_id.as_bytes());

    let mut mac = [0u8; 6];
    mac.copy_from_slice(&result.as_ref()[0..6]);

    // Set the locally administered bit and clear the multicast bit
    // Bit 1 = locally administered, Bit 0 = unicast/multicast
    mac[0] = (mac[0] & 0xfe) | 0x02;

    mac
}

pub fn format_mac_address(mac: &[u8; 6]) -> String {
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
        let mac = generate_mac_address("test-vm-id");

        // ASSERT
        assert_eq!(mac[0] & 0x02, 0x02);
    }

    #[test]
    fn generate_mac_clears_multicast_bit() {
        // ARRANGE / ACT
        let mac = generate_mac_address("test-vm-id");

        // ASSERT
        assert_eq!(mac[0] & 0x01, 0x00);
    }

    #[test]
    fn generate_mac_is_deterministic() {
        // ACT
        let a = generate_mac_address("same-id");
        let b = generate_mac_address("same-id");

        // ASSERT
        assert_eq!(a, b);
    }

    #[test]
    fn generate_mac_differs_for_different_ids() {
        // ACT
        let a = generate_mac_address("vm-alpha");
        let b = generate_mac_address("vm-beta");

        // ASSERT
        assert_ne!(a, b);
    }

    #[test]
    fn generate_mac_empty_id_is_valid() {
        // ACT
        let mac = generate_mac_address("");

        // ASSERT
        assert_eq!(mac[0] & 0x02, 0x02);
        assert_eq!(mac[0] & 0x01, 0x00);
    }

    #[test]
    fn format_mac_produces_colon_separated_hex() {
        // ARRANGE
        let mac = [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];

        // ACT / ASSERT
        assert_eq!(format_mac_address(&mac), "aa:bb:cc:dd:ee:ff");
    }

    #[test]
    fn format_mac_zero_pads_single_digit_bytes() {
        // ARRANGE
        let mac = [0x02, 0x00, 0x0a, 0x00, 0x00, 0x01];

        // ACT / ASSERT
        assert_eq!(format_mac_address(&mac), "02:00:0a:00:00:01");
    }

    #[test]
    fn format_mac_all_zeros() {
        // ARRANGE
        let mac = [0x00; 6];

        // ACT / ASSERT
        assert_eq!(format_mac_address(&mac), "00:00:00:00:00:00");
    }

    #[test]
    fn format_mac_all_ff() {
        // ARRANGE
        let mac = [0xff; 6];

        // ACT / ASSERT
        assert_eq!(format_mac_address(&mac), "ff:ff:ff:ff:ff:ff");
    }

    #[test]
    fn generate_then_format_roundtrip() {
        // ACT
        let mac = generate_mac_address("roundtrip-test");
        let formatted = format_mac_address(&mac);

        // ASSERT
        assert_eq!(formatted.len(), 17);
        assert_eq!(formatted.matches(':').count(), 5);
    }
}
