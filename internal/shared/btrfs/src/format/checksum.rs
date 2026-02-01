/// Compute Btrfs name hash matching the kernel's `btrfs_name_hash()`.
pub fn btrfs_name_hash(name: &[u8]) -> u64 {
    (crc32c::crc32c_append(1, name) ^ 0xFFFF_FFFF) as u64
}

/// Compute checksum and write to buffer as little-endian
pub fn compute_checksum(data: &[u8]) -> [u8; 4] {
    crc32c::crc32c(data).to_le_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_btrfs_name_hash_default() {
        assert_eq!(btrfs_name_hash(b"default"), 0x8dbfc2d2);
    }
}
