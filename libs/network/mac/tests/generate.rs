//! Deterministic MAC generation tests.

#![cfg(feature = "generate")]

#[cfg(test)]
mod tests {
    use mac::generate;

    #[test]
    fn sets_locally_administered_bit() {
        // ARRANGE / ACT
        let mac = generate("test-id");

        // ASSERT
        assert_eq!(mac.as_bytes()[0] & 0x02, 0x02);
    }

    #[test]
    fn clears_multicast_bit() {
        // ARRANGE / ACT
        let mac = generate("test-id");

        // ASSERT
        assert_eq!(mac.as_bytes()[0] & 0x01, 0x00);
    }

    #[test]
    fn is_deterministic() {
        // ACT
        let first_mac = generate("same-id");
        let second_mac = generate("same-id");

        // ASSERT
        assert_eq!(first_mac, second_mac);
    }

    #[test]
    fn differs_for_different_ids() {
        // ACT
        let first_mac = generate("alpha");
        let second_mac = generate("beta");

        // ASSERT
        assert_ne!(first_mac, second_mac);
    }

    #[test]
    fn empty_id_is_valid() {
        // ACT
        let mac = generate("");

        // ASSERT
        assert_eq!(mac.as_bytes()[0] & 0x02, 0x02);
        assert_eq!(mac.as_bytes()[0] & 0x01, 0x00);
    }
}
