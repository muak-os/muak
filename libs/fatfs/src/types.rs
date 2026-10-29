//! Core types and constants for FAT filesystem operations.

use crate::tree;

/// Sector size in bytes used for all FAT layouts.
pub(crate) const SECTOR_SIZE: u64 = 512;
/// Number of FAT copies written to disk.
pub(crate) const FAT_COUNT: u64 = 2;
/// First usable data cluster number.
pub(crate) const ROOT_CLUSTER: u32 = 2;

/// End-of-chain marker for FAT32.
pub(crate) const FAT32_EOC: u32 = 0x0FFF_FFFF;
/// Sentinel for an unknown cluster number in the `FSInfo` sector.
pub(crate) const UNKNOWN_CLUSTER: u32 = 0xFFFF_FFFF;

/// Directory entry attribute bit for directories.
pub(crate) const ATTR_DIRECTORY: u8 = 0x10;
/// Directory entry attribute bit for archive.
pub(crate) const ATTR_ARCHIVE: u8 = 0x20;
/// Directory entry attribute value for LFN entries.
pub(crate) const ATTR_LFN: u8 = 0x0F;
/// Directory entry attribute value for the volume label.
pub(crate) const ATTR_VOLUME_ID: u8 = 0x08;

/// Default volume serial number.
pub(crate) const VOLUME_ID: u32 = 0x1234_5678;
/// Volume label shared by the boot sector and the root directory entry.
pub(crate) const VOLUME_LABEL: [u8; 11] = *b"MUAKOS     ";

/// Sector holding the `FSInfo` structure.
pub(crate) const FSINFO_SECTOR: u16 = 1;
/// Sector holding the backup copy of the boot sector.
pub(crate) const BACKUP_BOOT_SECTOR: u16 = 6;
/// Smallest reserved region that holds the boot sector, `FSInfo` and its backup.
pub(crate) const MIN_RESERVED_SECTORS: u64 = 7;

/// Minimum cluster count for FAT32.
pub(crate) const FAT32_MIN_CLUSTERS: u64 = 0xFFF5;
/// Sectors reserved before the first FAT copy.
pub(crate) const RESERVED_SECTORS: u64 = 8;
/// Bytes per FAT32 table entry.
pub(crate) const FAT_ENTRY_SIZE: u64 = 4;

const _: () = assert!(
    RESERVED_SECTORS >= MIN_RESERVED_SECTORS,
    "the reserved region must hold the boot sector, FSInfo and its backup"
);
const _: () = assert!(
    FSINFO_SECTOR < BACKUP_BOOT_SECTOR,
    "the FSInfo sector must precede the backup boot sector"
);

/// Smallest total image size in bytes that the builder can format as FAT32.
pub const MIN_IMAGE_SIZE: u64 = min_image_size();

/// Largest total image size in bytes the builder can format as FAT32.
pub const MAX_IMAGE_SIZE: u64 = 0xFFFF_FFFF_u64.saturating_mul(SECTOR_SIZE);

/// Metadata for a file in a FAT filesystem.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileMeta<'a> {
    /// Path of the file relative to the filesystem root.
    pub path: &'a str,
    /// Size of the file in bytes.
    pub size: u64,
}

impl<'a> FileMeta<'a> {
    /// Creates a new `FileMeta` with the given path and size.
    #[must_use]
    pub const fn new(path: &'a str, size: u64) -> Self {
        Self { path, size }
    }
}

/// Precomputed FAT filesystem metadata.
#[derive(Debug)]
pub struct Precomputed<'a> {
    pub(crate) layout: FatLayout,
    pub(crate) tree: tree::DirIndex<'a>,
    pub(crate) cluster_map: ClusterMap,
    pub(crate) fat_bytes: Vec<u8>,
    pub(crate) dir_data: Vec<Vec<u8>>,
    pub(crate) fs_info: FsInfo,
    pub(crate) image_size: u64,
}

/// Free space accounting written to the `FSInfo` sector.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FsInfo {
    pub(crate) free_clusters: u32,
    pub(crate) next_free: u32,
}

impl FsInfo {
    /// Derives the free space accounting from an allocated cluster map.
    pub(crate) fn from_map(map: &ClusterMap, layout: &FatLayout) -> Self {
        let dirs = map.dir_counts.iter().copied().map(u64::from).sum::<u64>();
        let files = map.file_counts.iter().copied().sum::<u64>();
        let used = dirs.saturating_add(files);
        let free = layout.data_cluster_count.saturating_sub(used);
        let next_free = if free == 0 {
            UNKNOWN_CLUSTER
        } else {
            u32::try_from(u64::from(ROOT_CLUSTER).saturating_add(used)).unwrap_or(UNKNOWN_CLUSTER)
        };

        Self {
            free_clusters: u32::try_from(free).unwrap_or(UNKNOWN_CLUSTER),
            next_free,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct FatLayout {
    pub total_sectors: u64,
    pub reserved_sectors: u64,
    pub fat_sectors: u64,
    pub spc: u64,
    pub data_cluster_count: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct ClusterMap {
    pub dir_starts: Vec<u32>,
    pub dir_counts: Vec<u32>,
    pub file_starts: Vec<u32>,
    pub file_counts: Vec<u64>,
    pub file_sizes: Vec<u64>,
}

const fn min_image_size() -> u64 {
    let entries_per_sector = SECTOR_SIZE.div_euclid(FAT_ENTRY_SIZE);
    let fat_sectors = FAT32_MIN_CLUSTERS
        .saturating_add(2)
        .div_ceil(entries_per_sector.saturating_sub(FAT_COUNT));
    let total_sectors = FAT32_MIN_CLUSTERS
        .saturating_add(RESERVED_SECTORS)
        .saturating_add(FAT_COUNT.saturating_mul(fat_sectors));

    total_sectors.saturating_mul(SECTOR_SIZE)
}
