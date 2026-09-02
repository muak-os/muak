use uuid::Uuid;

use super::structures::BtrfsDiskKey;

/// Write u64 as little-endian to byte array.
pub fn write_u64(dest: &mut [u8; 8], value: u64) {
    dest.copy_from_slice(&value.to_le_bytes());
}

/// Write u32 as little-endian to byte array.
pub fn write_u32(dest: &mut [u8; 4], value: u32) {
    dest.copy_from_slice(&value.to_le_bytes());
}

/// Write u16 as little-endian to byte array.
pub fn write_u16(dest: &mut [u8; 2], value: u16) {
    dest.copy_from_slice(&value.to_le_bytes());
}

/// Write UUID to byte array.
pub fn write_uuid(dest: &mut [u8; 16], uuid: &Uuid) {
    dest.copy_from_slice(uuid.as_bytes());
}

/// Helper to write disk key.
pub fn write_disk_key(dest: &mut BtrfsDiskKey, objectid: u64, type_: u8, offset: u64) {
    write_u64(&mut dest.objectid, objectid);
    dest.type_ = type_;
    write_u64(&mut dest.offset, offset);
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    #[test]
    fn write_u64_stores_little_endian() {
        // ARRANGE
        let mut dest = [0_u8; 8];

        // ACT
        write_u64(&mut dest, 0x0102_0304_0506_0708);

        // ASSERT
        assert_eq!(dest, [8, 7, 6, 5, 4, 3, 2, 1]);
    }

    #[test]
    fn write_u32_stores_little_endian() {
        // ARRANGE
        let mut dest = [0_u8; 4];

        // ACT
        write_u32(&mut dest, 0x0102_0304);

        // ASSERT
        assert_eq!(dest, [4, 3, 2, 1]);
    }

    #[test]
    fn write_u16_stores_little_endian() {
        // ARRANGE
        let mut dest = [0_u8; 2];

        // ACT
        write_u16(&mut dest, 0x0102);

        // ASSERT
        assert_eq!(dest, [2, 1]);
    }

    #[test]
    fn write_uuid_copies_all_16_bytes() {
        // ARRANGE
        let uuid = Uuid::from_u128(0x0102_0304_0506_0708_090A_0B0C_0D0E_0F10);
        let mut dest = [0_u8; 16];

        // ACT
        write_uuid(&mut dest, &uuid);

        // ASSERT
        assert_eq!(dest, *uuid.as_bytes());
    }

    #[test]
    fn write_disk_key_fills_objectid_type_and_offset() {
        // ARRANGE
        let mut key = BtrfsDiskKey {
            objectid: [0; 8],
            type_: 0,
            offset: [0; 8],
        };

        // ACT
        write_disk_key(&mut key, 256, 228, u64::MAX);

        // ASSERT
        assert_eq!(u64::from_le_bytes(key.objectid), 256);
        assert_eq!(key.type_, 228);
        assert_eq!(u64::from_le_bytes(key.offset), u64::MAX);
    }
}
