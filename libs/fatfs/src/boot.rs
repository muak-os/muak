//! FAT32 boot sector and `FSInfo` encoding.

use crate::types::{
    BACKUP_BOOT_SECTOR, FAT_COUNT, FSINFO_SECTOR, FatLayout, FsInfo, ROOT_CLUSTER, SECTOR_SIZE,
    VOLUME_ID, VOLUME_LABEL,
};

/// Bytes in one sector.
const SECTOR_BYTES: usize = 512;
/// Bytes per track reported in the BPB.
const SECTORS_PER_TRACK: u16 = 32;
/// Heads reported in the BPB.
const HEADS: u16 = 64;
/// Hard disk media descriptor.
const MEDIA: u8 = 0xF8;
/// BIOS drive number reported in the BPB.
const DRIVE_NUMBER: u8 = 0x80;
/// Extended boot signature announcing the FAT32 fields.
const EXTENDED_SIGNATURE: u8 = 0x29;
/// `FSInfo` lead signature.
const FSINFO_LEAD: u32 = 0x4161_5252;
/// `FSInfo` structure signature.
const FSINFO_STRUCT: u32 = 0x6141_7272;
/// `FSInfo` trail signature, including the boot sector signature.
const FSINFO_TRAIL: u32 = 0xAA55_0000;
/// Boot code printed when the volume is booted by a BIOS.
const BOOT_CODE: [u8; 129] = [
    0x0E, 0x1F, 0xBE, 0x77, 0x7C, 0xAC, 0x22, 0xC0, 0x74, 0x0B, 0x56, 0xB4, 0x0E, 0xBB, 0x07, 0x00,
    0xCD, 0x10, 0x5E, 0xEB, 0xF0, 0x32, 0xE4, 0xCD, 0x16, 0xCD, 0x19, 0xEB, 0xFE, 0x54, 0x68, 0x69,
    0x73, 0x20, 0x69, 0x73, 0x20, 0x6E, 0x6F, 0x74, 0x20, 0x61, 0x20, 0x62, 0x6F, 0x6F, 0x74, 0x61,
    0x62, 0x6C, 0x65, 0x20, 0x64, 0x69, 0x73, 0x6B, 0x2E, 0x20, 0x20, 0x50, 0x6C, 0x65, 0x61, 0x73,
    0x65, 0x20, 0x69, 0x6E, 0x73, 0x65, 0x72, 0x74, 0x20, 0x61, 0x20, 0x62, 0x6F, 0x6F, 0x74, 0x61,
    0x62, 0x6C, 0x65, 0x20, 0x66, 0x6C, 0x6F, 0x70, 0x70, 0x79, 0x20, 0x61, 0x6E, 0x64, 0x0D, 0x0A,
    0x70, 0x72, 0x65, 0x73, 0x73, 0x20, 0x61, 0x6E, 0x79, 0x20, 0x6B, 0x65, 0x79, 0x20, 0x74, 0x6F,
    0x20, 0x74, 0x72, 0x79, 0x20, 0x61, 0x67, 0x61, 0x69, 0x6E, 0x20, 0x2E, 0x2E, 0x2E, 0x20, 0x0D,
    0x0A,
];

/// Encodes the boot sector of a FAT32 volume.
pub(crate) fn sector(layout: &FatLayout) -> [u8; SECTOR_BYTES] {
    let mut boot = [0_u8; SECTOR_BYTES];
    put(&mut boot, 0, &[0xEB, 0x58, 0x90]);
    put(&mut boot, 3, b"MSWIN4.1");
    put(
        &mut boot,
        11,
        &u16::try_from(SECTOR_SIZE).unwrap_or(0).to_le_bytes(),
    );
    put(&mut boot, 13, &[u8::try_from(layout.spc).unwrap_or(1)]);
    put(
        &mut boot,
        14,
        &u16::try_from(layout.reserved_sectors)
            .unwrap_or(0)
            .to_le_bytes(),
    );
    put(&mut boot, 16, &[u8::try_from(FAT_COUNT).unwrap_or(0)]);
    put(&mut boot, 21, &[MEDIA]);
    put(&mut boot, 24, &SECTORS_PER_TRACK.to_le_bytes());
    put(&mut boot, 26, &HEADS.to_le_bytes());
    put(
        &mut boot,
        32,
        &u32::try_from(layout.total_sectors)
            .unwrap_or(0)
            .to_le_bytes(),
    );
    put(
        &mut boot,
        36,
        &u32::try_from(layout.fat_sectors).unwrap_or(0).to_le_bytes(),
    );
    put(&mut boot, 44, &ROOT_CLUSTER.to_le_bytes());
    put(&mut boot, 48, &FSINFO_SECTOR.to_le_bytes());
    put(&mut boot, 50, &BACKUP_BOOT_SECTOR.to_le_bytes());
    put(&mut boot, 64, &[DRIVE_NUMBER]);
    put(&mut boot, 66, &[EXTENDED_SIGNATURE]);
    put(&mut boot, 67, &VOLUME_ID.to_le_bytes());
    put(&mut boot, 71, &VOLUME_LABEL);
    put(&mut boot, 82, b"FAT32   ");
    put(&mut boot, 90, &BOOT_CODE);
    put(&mut boot, 510, &[0x55, 0xAA]);

    boot
}

/// Encodes the `FSInfo` sector of a FAT32 volume.
pub(crate) fn fsinfo(info: FsInfo) -> [u8; SECTOR_BYTES] {
    let mut sector = [0_u8; SECTOR_BYTES];
    put(&mut sector, 0, &FSINFO_LEAD.to_le_bytes());
    put(&mut sector, 484, &FSINFO_STRUCT.to_le_bytes());
    put(&mut sector, 488, &info.free_clusters.to_le_bytes());
    put(&mut sector, 492, &info.next_free.to_le_bytes());
    put(&mut sector, 508, &FSINFO_TRAIL.to_le_bytes());

    sector
}

fn put(buf: &mut [u8], offset: usize, src: &[u8]) {
    let end = offset.saturating_add(src.len());
    if let Some(slot) = buf.get_mut(offset..end) {
        slot.copy_from_slice(src);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FAT_ENTRY_SIZE;

    fn sample_layout() -> FatLayout {
        FatLayout {
            total_sectors: 139_528,
            reserved_sectors: 8,
            fat_sectors: 546,
            spc: 2,
            data_cluster_count: 69_214,
        }
    }

    fn read_u16(buf: &[u8], offset: usize) -> u16 {
        buf.get(offset..offset.saturating_add(2))
            .and_then(|bytes| bytes.try_into().ok())
            .map_or(0, u16::from_le_bytes)
    }

    fn read_u32(buf: &[u8], offset: usize) -> u32 {
        buf.get(offset..offset.saturating_add(4))
            .and_then(|bytes| bytes.try_into().ok())
            .map_or(0, u32::from_le_bytes)
    }

    #[test]
    fn boot_sector_describes_the_volume() {
        // ARRANGE
        let layout = sample_layout();

        // ACT
        let boot = sector(&layout);

        // ASSERT
        assert_eq!(boot.get(0..3), Some(&[0xEB, 0x58, 0x90][..]), "jump");
        assert_eq!(boot.get(3..11), Some(&b"MSWIN4.1"[..]), "OEM ID");
        assert_eq!(read_u16(&boot, 11), 512, "bytes per sector");
        assert_eq!(boot.get(13), Some(&2), "sectors per cluster");
        assert_eq!(read_u16(&boot, 14), 8, "reserved sectors");
        assert_eq!(boot.get(16), Some(&2), "FAT copies");
        assert_eq!(read_u16(&boot, 17), 0, "root entries (FAT32)");
        assert_eq!(boot.get(21), Some(&MEDIA), "media descriptor");
        assert_eq!(read_u16(&boot, 22), 0, "FAT size 16 (FAT32)");
        assert_eq!(read_u32(&boot, 32), 139_528, "total sectors");
        assert_eq!(read_u32(&boot, 36), 546, "FAT size 32");
        assert_eq!(read_u32(&boot, 44), ROOT_CLUSTER, "root cluster");
        assert_eq!(read_u16(&boot, 48), FSINFO_SECTOR, "FSInfo sector");
        assert_eq!(read_u16(&boot, 50), BACKUP_BOOT_SECTOR, "backup sector");
        assert_eq!(boot.get(64), Some(&DRIVE_NUMBER), "drive number");
        assert_eq!(boot.get(66), Some(&EXTENDED_SIGNATURE), "boot signature");
        assert_eq!(read_u32(&boot, 67), VOLUME_ID, "volume id");
        assert_eq!(boot.get(71..82), Some(&VOLUME_LABEL[..]), "volume label");
        assert_eq!(boot.get(82..90), Some(&b"FAT32   "[..]), "filesystem type");
        assert_eq!(
            boot.get(510..512),
            Some(&[0x55, 0xAA][..]),
            "boot signature"
        );
    }

    #[test]
    fn boot_sector_geometry_matches_the_layout() {
        // ARRANGE
        let layout = sample_layout();

        // ACT
        let boot = sector(&layout);

        // ASSERT
        let reserved = u64::from(read_u16(&boot, 14));
        let fats = u64::from(read_u32(&boot, 36)).saturating_mul(2);
        let clusters = u64::from(read_u32(&boot, 32))
            .saturating_sub(reserved)
            .saturating_sub(fats)
            .div_euclid(2);
        assert_eq!(clusters, layout.data_cluster_count, "data clusters");
        assert!(
            u64::from(read_u32(&boot, 36)).saturating_mul(SECTOR_SIZE)
                >= layout.data_cluster_count.saturating_mul(FAT_ENTRY_SIZE),
            "the FAT must hold one entry per data cluster"
        );
    }

    #[test]
    fn fsinfo_carries_the_free_space_accounting() {
        // ARRANGE
        let info = FsInfo {
            free_clusters: 4_096,
            next_free: 65_118,
        };

        // ACT
        let encoded = fsinfo(info);

        // ASSERT
        assert_eq!(read_u32(&encoded, 0), FSINFO_LEAD, "lead signature");
        assert_eq!(
            read_u32(&encoded, 484),
            FSINFO_STRUCT,
            "structure signature"
        );
        assert_eq!(read_u32(&encoded, 488), 4_096, "free clusters");
        assert_eq!(read_u32(&encoded, 492), 65_118, "next free cluster");
        assert_eq!(read_u32(&encoded, 508), FSINFO_TRAIL, "trail signature");
    }

    #[test]
    fn backup_sector_fits_in_the_reserved_region() {
        // ARRANGE
        let layout = sample_layout();

        // ACT
        let needed = u64::from(BACKUP_BOOT_SECTOR).saturating_add(1);

        // ASSERT
        assert!(
            layout.reserved_sectors >= needed,
            "the reserved region must hold the backup boot sector"
        );
    }
}
