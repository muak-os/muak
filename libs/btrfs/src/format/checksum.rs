/// Compute Btrfs name hash matching the kernel's `btrfs_name_hash()`.
pub fn btrfs_name_hash(name: &[u8]) -> u64 {
    u64::from(crc32c::crc32c_append(1, name) ^ 0xFFFF_FFFF)
}

/// Compute checksum and write to buffer as little-endian.
pub fn compute_checksum(data: &[u8]) -> [u8; 4] {
    crc32c::crc32c(data).to_le_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn btrfs_name_hash_default() {
        // ARRANGE
        let name = b"default";

        // ACT
        let hash = btrfs_name_hash(name);

        // ASSERT
        assert_eq!(hash, 0x8dbf_c2d2);
    }

    #[test]
    fn btrfs_name_hash_of_empty_name_is_complement_of_seed() {
        // ARRANGE
        let name: &[u8] = b"";

        // ACT
        let hash = btrfs_name_hash(name);

        // ASSERT
        assert_eq!(hash, 0xFFFF_FFFE);
    }

    #[test]
    fn compute_checksum_matches_crc32c_reference_vector() {
        // ARRANGE
        let data = b"123456789";

        // ACT
        let checksum = compute_checksum(data);

        // ASSERT
        assert_eq!(checksum, [0x83, 0x92, 0x06, 0xE3]);
    }

    #[test]
    fn compute_checksum_of_empty_input_is_zero() {
        // ARRANGE
        let data: &[u8] = b"";

        // ACT
        let checksum = compute_checksum(data);

        // ASSERT
        assert_eq!(checksum, [0, 0, 0, 0]);
    }
}
