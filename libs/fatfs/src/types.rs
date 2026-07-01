//! Core types and constants for FAT filesystem operations.

/// Sector size in bytes used for all FAT layouts.
pub(crate) const SECTOR_SIZE: u64 = 512;
/// Number of FAT copies written to disk.
pub(crate) const FAT_COUNT: u64 = 2;
/// First usable data cluster number.
pub(crate) const ROOT_CLUSTER: u32 = 2;

/// End-of-chain marker for FAT12.
pub(crate) const FAT12_EOC: u32 = 0x0FFF;
/// End-of-chain marker for FAT16.
pub(crate) const FAT16_EOC: u32 = 0xFFFF;
/// End-of-chain marker for FAT32.
pub(crate) const FAT32_EOC: u32 = 0x0FFF_FFFF;

/// Directory entry attribute bit for directories.
pub(crate) const ATTR_DIRECTORY: u8 = 0x10;
/// Directory entry attribute bit for archive.
pub(crate) const ATTR_ARCHIVE: u8 = 0x20;
/// Directory entry attribute value for LFN entries.
pub(crate) const ATTR_LFN: u8 = 0x0F;

/// Default volume serial number.
pub(crate) const VOLUME_ID: u32 = 0x1234_5678;

/// Minimum cluster count for FAT32.
pub(crate) const FAT32_MIN_CLUSTERS: u64 = 65525;
/// Minimum cluster count for FAT16.
pub(crate) const FAT16_MIN_CLUSTERS: u64 = 4085;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum FatKind {
    Fat12,
    Fat16,
    Fat32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct FatLayout {
    pub total_sectors: u64,
    pub reserved_sectors: u64,
    pub fat_sectors: u64,
    pub spc: u64,
    pub root_dir_sectors: u64,
    pub data_cluster_count: u64,
    pub kind: FatKind,
}

#[derive(Clone, Debug)]
pub(crate) struct ClusterMap {
    pub dir_clusters: Vec<u32>,
    pub file_starts: Vec<u32>,
    pub file_counts: Vec<u64>,
}

pub(crate) fn fat32_cluster(index: usize) -> u32 {
    u32::try_from(index).map_or(ROOT_CLUSTER, |idx| ROOT_CLUSTER.wrapping_add(idx))
}

pub(crate) fn fat12_16_cluster(index: usize) -> u32 {
    if index == 0 {
        return 0;
    }
    ROOT_CLUSTER
        .wrapping_add(u32::try_from(index).unwrap_or(0))
        .wrapping_sub(1)
}

/// Trait for readable file sources used by the FAT image builder.
pub trait FileSource {
    /// The relative path of the file within the FAT volume.
    fn path(&self) -> &str;

    /// The exact byte size of the file content.
    fn size(&self) -> u64;

    /// Read bytes into the buffer, returning the number read.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when reading from the underlying source fails.
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize>;
}
