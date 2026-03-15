/// Write u64 as little-endian to byte array
pub fn write_u64(dest: &mut [u8; 8], value: u64) {
    dest.copy_from_slice(&value.to_le_bytes());
}

/// Write u32 as little-endian to byte array
pub fn write_u32(dest: &mut [u8; 4], value: u32) {
    dest.copy_from_slice(&value.to_le_bytes());
}

/// Write u16 as little-endian to byte array
pub fn write_u16(dest: &mut [u8; 2], value: u16) {
    dest.copy_from_slice(&value.to_le_bytes());
}

/// Write UUID to byte array
pub fn write_uuid(dest: &mut [u8; 16], uuid: &uuid::Uuid) {
    dest.copy_from_slice(uuid.as_bytes());
}

/// Helper to write disk key
pub fn write_disk_key(
    dest: &mut super::structures::BtrfsDiskKey,
    objectid: u64,
    type_: u8,
    offset: u64,
) {
    write_u64(&mut dest.objectid, objectid);
    dest.type_ = type_;
    write_u64(&mut dest.offset, offset);
}
