use core::mem::size_of;
use core::ptr;
use core::slice;

/// Bytes reserved for a checksum in Btrfs headers.
pub(super) const BTRFS_CSUM_SIZE: usize = 32;

/// Maximum bytes stored for the filesystem label.
pub(super) const BTRFS_LABEL_SIZE: usize = 256;

/// Bytes stored for a UUID on disk.
pub(super) const BTRFS_UUID_SIZE: usize = 16;

/// Bytes stored for a filesystem UUID on disk.
const BTRFS_FSID_SIZE: usize = 16;

/// Serialized size of a Btrfs superblock.
const BTRFS_SUPER_INFO_SIZE: usize = 4096;

/// Bytes reserved for the superblock system chunk array.
const BTRFS_SYSTEM_CHUNK_ARRAY_SIZE: usize = 2048;

/// Safe byte serialization for `#[repr(C, packed)]` on-disk structures.
pub trait AsBytes: Sized {
    fn as_bytes(&self) -> &[u8] {
        as_bytes(self, size_of::<Self>())
    }

    fn to_vec(&self) -> Vec<u8> {
        self.as_bytes().to_vec()
    }
}

fn as_bytes<T>(value: &T, len: usize) -> &[u8] {
    let ptr = ptr::from_ref(value).cast::<u8>();

    // SAFETY: Callers only pass references to `repr(C, packed)` on-disk structures and a
    // length that does not exceed the backing object size.
    unsafe { slice::from_raw_parts(ptr, len) }
}

/// Disk key for btrfs items.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct BtrfsDiskKey {
    pub objectid: [u8; 8],
    pub type_: u8,
    pub offset: [u8; 8],
}

impl AsBytes for BtrfsDiskKey {}

/// Btrfs item header in leaf nodes.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct BtrfsItem {
    pub key: BtrfsDiskKey,
    pub offset: [u8; 4],
    pub size: [u8; 4],
}

impl AsBytes for BtrfsItem {}

/// Common header for all tree nodes.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct BtrfsHeader {
    pub csum: [u8; BTRFS_CSUM_SIZE],
    pub fsid: [u8; BTRFS_FSID_SIZE],
    pub bytenr: [u8; 8],
    pub flags: [u8; 8],
    pub chunk_tree_uuid: [u8; BTRFS_UUID_SIZE],
    pub generation: [u8; 8],
    pub owner: [u8; 8],
    pub nritems: [u8; 4],
    pub level: u8,
}

impl BtrfsHeader {
    #[must_use]
    pub fn new() -> Self {
        Self {
            csum: [0; BTRFS_CSUM_SIZE],
            fsid: [0; BTRFS_FSID_SIZE],
            bytenr: [0; 8],
            flags: [0; 8],
            chunk_tree_uuid: [0; BTRFS_UUID_SIZE],
            generation: [0; 8],
            owner: [0; 8],
            nritems: [0; 4],
            level: 0,
        }
    }
}

impl Default for BtrfsHeader {
    fn default() -> Self {
        Self::new()
    }
}

impl AsBytes for BtrfsHeader {}

/// Device item.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct BtrfsDevItem {
    pub devid: [u8; 8],
    pub total_bytes: [u8; 8],
    pub bytes_used: [u8; 8],
    pub io_align: [u8; 4],
    pub io_width: [u8; 4],
    pub sector_size: [u8; 4],
    pub type_: [u8; 8],
    pub generation: [u8; 8],
    pub start_offset: [u8; 8],
    pub dev_group: [u8; 4],
    pub seek_speed: u8,
    pub bandwidth: u8,
    pub uuid: [u8; BTRFS_UUID_SIZE],
    pub fsid: [u8; BTRFS_FSID_SIZE],
}

impl BtrfsDevItem {
    #[must_use]
    pub fn new() -> Self {
        Self {
            devid: [0; 8],
            total_bytes: [0; 8],
            bytes_used: [0; 8],
            io_align: [0; 4],
            io_width: [0; 4],
            sector_size: [0; 4],
            type_: [0; 8],
            generation: [0; 8],
            start_offset: [0; 8],
            dev_group: [0; 4],
            seek_speed: 0,
            bandwidth: 0,
            uuid: [0; BTRFS_UUID_SIZE],
            fsid: [0; BTRFS_FSID_SIZE],
        }
    }
}

impl Default for BtrfsDevItem {
    fn default() -> Self {
        Self::new()
    }
}

impl AsBytes for BtrfsDevItem {}

/// Stripe within a chunk.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct BtrfsStripe {
    pub devid: [u8; 8],
    pub offset: [u8; 8],
    pub dev_uuid: [u8; BTRFS_UUID_SIZE],
}

impl AsBytes for BtrfsStripe {}

/// Chunk item.
#[repr(C, packed)]
#[derive(Debug)]
pub struct BtrfsChunk {
    pub length: [u8; 8],
    pub owner: [u8; 8],
    pub stripe_len: [u8; 8],
    pub type_: [u8; 8],
    pub io_align: [u8; 4],
    pub io_width: [u8; 4],
    pub sector_size: [u8; 4],
    pub num_stripes: [u8; 2],
    pub sub_stripes: [u8; 2],
}

impl AsBytes for BtrfsChunk {}

/// Block group item.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct BtrfsBlockGroupItem {
    pub used: [u8; 8],
    pub chunk_objectid: [u8; 8],
    pub flags: [u8; 8],
}

impl AsBytes for BtrfsBlockGroupItem {}

/// Timespec.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct BtrfsTimespec {
    pub sec: [u8; 8],
    pub nsec: [u8; 4],
}

impl AsBytes for BtrfsTimespec {}

/// Inode item.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct BtrfsInodeItem {
    pub generation: [u8; 8],
    pub transid: [u8; 8],
    pub size: [u8; 8],
    pub nbytes: [u8; 8],
    pub block_group: [u8; 8],
    pub nlink: [u8; 4],
    pub uid: [u8; 4],
    pub gid: [u8; 4],
    pub mode: [u8; 4],
    pub rdev: [u8; 8],
    pub flags: [u8; 8],
    pub sequence: [u8; 8],
    pub reserved: [u8; 32],
    pub atime: BtrfsTimespec,
    pub ctime: BtrfsTimespec,
    pub mtime: BtrfsTimespec,
    pub otime: BtrfsTimespec,
}

impl AsBytes for BtrfsInodeItem {}

/// Root item.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct BtrfsRootItem {
    pub inode: BtrfsInodeItem,
    pub generation: [u8; 8],
    pub root_dirid: [u8; 8],
    pub bytenr: [u8; 8],
    pub byte_limit: [u8; 8],
    pub bytes_used: [u8; 8],
    pub last_snapshot: [u8; 8],
    pub flags: [u8; 8],
    pub refs: [u8; 4],
    pub drop_progress: BtrfsDiskKey,
    pub drop_level: u8,
    pub level: u8,
    pub generation_v2: [u8; 8],
    pub uuid: [u8; BTRFS_UUID_SIZE],
    pub parent_uuid: [u8; BTRFS_UUID_SIZE],
    pub received_uuid: [u8; BTRFS_UUID_SIZE],
    pub ctransid: [u8; 8],
    pub otransid: [u8; 8],
    pub stransid: [u8; 8],
    pub rtransid: [u8; 8],
    pub ctime: BtrfsTimespec,
    pub otime: BtrfsTimespec,
    pub stime: BtrfsTimespec,
    pub rtime: BtrfsTimespec,
    pub reserved: [u8; 64],
}

impl AsBytes for BtrfsRootItem {}

/// Inode reference.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct BtrfsInodeRef {
    pub index: [u8; 8],
    pub name_len: [u8; 2],
}

impl AsBytes for BtrfsInodeRef {}

/// Extent item.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct BtrfsExtentItem {
    pub refs: [u8; 8],
    pub generation: [u8; 8],
    pub flags: [u8; 8],
}

impl AsBytes for BtrfsExtentItem {}

/// Device extent.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct BtrfsDevExtent {
    pub chunk_tree: [u8; 8],
    pub chunk_objectid: [u8; 8],
    pub chunk_offset: [u8; 8],
    pub length: [u8; 8],
    pub chunk_tree_uuid: [u8; BTRFS_UUID_SIZE],
}

impl AsBytes for BtrfsDevExtent {}

/// Free space info.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct BtrfsFreeSpaceInfo {
    pub extent_count: [u8; 4],
    pub flags: [u8; 4],
}

impl AsBytes for BtrfsFreeSpaceInfo {}

/// Device stats item.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct BtrfsDevStatsItem {
    pub values: [[u8; 8]; 5],
}

impl AsBytes for BtrfsDevStatsItem {}

/// Directory item (fixed-size portion, name follows).
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct BtrfsDirItem {
    pub location: BtrfsDiskKey,
    pub transid: [u8; 8],
    pub data_len: [u8; 2],
    pub name_len: [u8; 2],
    pub type_: u8,
}

impl AsBytes for BtrfsDirItem {}

/// Superblock structure (4096 bytes).
#[repr(C, packed)]
pub struct BtrfsSuperBlock {
    pub csum: [u8; BTRFS_CSUM_SIZE],
    pub fsid: [u8; BTRFS_FSID_SIZE],
    pub bytenr: [u8; 8],
    pub flags: [u8; 8],
    pub magic: [u8; 8],
    pub generation: [u8; 8],
    pub root: [u8; 8],
    pub chunk_root: [u8; 8],
    pub log_root: [u8; 8],
    pub log_root_transid: [u8; 8],
    pub total_bytes: [u8; 8],
    pub bytes_used: [u8; 8],
    pub root_dir_objectid: [u8; 8],
    pub num_devices: [u8; 8],
    pub sectorsize: [u8; 4],
    pub nodesize: [u8; 4],
    pub leafsize: [u8; 4],
    pub stripesize: [u8; 4],
    pub sys_chunk_array_size: [u8; 4],
    pub chunk_root_generation: [u8; 8],
    pub compat_flags: [u8; 8],
    pub compat_ro_flags: [u8; 8],
    pub incompat_flags: [u8; 8],
    pub csum_type: [u8; 2],
    pub root_level: u8,
    pub chunk_root_level: u8,
    pub log_root_level: u8,
    pub dev_item: BtrfsDevItem,
    pub label: [u8; BTRFS_LABEL_SIZE],
    pub cache_generation: [u8; 8],
    pub uuid_tree_generation: [u8; 8],
    pub metadata_uuid: [u8; BTRFS_FSID_SIZE],
    pub nr_global_roots: [u8; 8],
    pub reserved: [u8; 216], // 27 * 8 = 216 bytes
    pub sys_chunk_array: [u8; BTRFS_SYSTEM_CHUNK_ARRAY_SIZE],
    pub super_roots: [u8; 672], // 4 * 168 = 672 bytes (btrfs_root_backup)
    pub padding: [u8; 565],
}

impl BtrfsSuperBlock {
    #[must_use]
    pub fn new() -> Self {
        Self {
            csum: [0; BTRFS_CSUM_SIZE],
            fsid: [0; BTRFS_FSID_SIZE],
            bytenr: [0; 8],
            flags: [0; 8],
            magic: [0; 8],
            generation: [0; 8],
            root: [0; 8],
            chunk_root: [0; 8],
            log_root: [0; 8],
            log_root_transid: [0; 8],
            total_bytes: [0; 8],
            bytes_used: [0; 8],
            root_dir_objectid: [0; 8],
            num_devices: [0; 8],
            sectorsize: [0; 4],
            nodesize: [0; 4],
            leafsize: [0; 4],
            stripesize: [0; 4],
            sys_chunk_array_size: [0; 4],
            chunk_root_generation: [0; 8],
            compat_flags: [0; 8],
            compat_ro_flags: [0; 8],
            incompat_flags: [0; 8],
            csum_type: [0; 2],
            root_level: 0,
            chunk_root_level: 0,
            log_root_level: 0,
            dev_item: BtrfsDevItem::new(),
            label: [0; BTRFS_LABEL_SIZE],
            cache_generation: [0; 8],
            uuid_tree_generation: [0; 8],
            metadata_uuid: [0; BTRFS_FSID_SIZE],
            nr_global_roots: [0; 8],
            reserved: [0; 216],
            sys_chunk_array: [0; BTRFS_SYSTEM_CHUNK_ARRAY_SIZE],
            super_roots: [0; 672],
            padding: [0; 565],
        }
    }
}

impl AsBytes for BtrfsSuperBlock {
    fn as_bytes(&self) -> &[u8] {
        as_bytes(self, BTRFS_SUPER_INFO_SIZE)
    }
}

impl Default for BtrfsSuperBlock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use core::mem::size_of;

    use super::*;

    #[test]
    fn on_disk_struct_sizes_match_btrfs_wire_format() {
        // ASSERT
        assert_eq!(size_of::<BtrfsDiskKey>(), 17);
        assert_eq!(size_of::<BtrfsItem>(), 25);
        assert_eq!(size_of::<BtrfsHeader>(), 101);
        assert_eq!(size_of::<BtrfsDevItem>(), 98);
        assert_eq!(size_of::<BtrfsStripe>(), 32);
        assert_eq!(size_of::<BtrfsChunk>(), 48);
        assert_eq!(size_of::<BtrfsBlockGroupItem>(), 24);
        assert_eq!(size_of::<BtrfsTimespec>(), 12);
        assert_eq!(size_of::<BtrfsInodeItem>(), 160);
        assert_eq!(size_of::<BtrfsRootItem>(), 439);
        assert_eq!(size_of::<BtrfsInodeRef>(), 10);
        assert_eq!(size_of::<BtrfsExtentItem>(), 24);
        assert_eq!(size_of::<BtrfsDevExtent>(), 48);
        assert_eq!(size_of::<BtrfsFreeSpaceInfo>(), 8);
        assert_eq!(size_of::<BtrfsDevStatsItem>(), 40);
        assert_eq!(size_of::<BtrfsDirItem>(), 30);
    }

    #[test]
    fn superblock_serializes_exactly_4096_bytes() {
        // ARRANGE
        let superblock = BtrfsSuperBlock::new();

        // ACT
        let bytes = superblock.as_bytes();

        // ASSERT
        assert_eq!(bytes.len(), BTRFS_SUPER_INFO_SIZE);
    }

    #[test]
    fn new_constructors_produce_zeroed_structures() {
        // ARRANGE
        let header = BtrfsHeader::new();
        let dev_item = BtrfsDevItem::new();
        let superblock = BtrfsSuperBlock::new();

        // ACT
        let header_bytes = header.as_bytes();
        let dev_item_bytes = dev_item.as_bytes();
        let superblock_bytes = superblock.as_bytes();

        // ASSERT
        assert!(header_bytes.iter().all(|&byte| byte == 0));
        assert!(dev_item_bytes.iter().all(|&byte| byte == 0));
        assert!(superblock_bytes.iter().all(|&byte| byte == 0));
    }

    #[test]
    fn disk_key_serializes_little_endian_fields() {
        // ARRANGE
        let mut key = BtrfsDiskKey {
            objectid: [0; 8],
            type_: 0,
            offset: [0; 8],
        };
        key.objectid.copy_from_slice(&7_u64.to_le_bytes());
        key.type_ = 84;
        key.offset.copy_from_slice(&9_u64.to_le_bytes());

        // ACT
        let bytes = key.as_bytes();

        // ASSERT
        assert_eq!(bytes.get(0..8), Some(7_u64.to_le_bytes().as_slice()));
        assert_eq!(bytes.get(8), Some(&84));
        assert_eq!(bytes.get(9..17), Some(9_u64.to_le_bytes().as_slice()));
    }

    #[test]
    fn to_vec_copies_serialized_bytes() {
        // ARRANGE
        let mut timespec = BtrfsTimespec {
            sec: [0; 8],
            nsec: [0; 4],
        };
        timespec.sec.copy_from_slice(&1_234_u64.to_le_bytes());

        // ACT
        let data = timespec.to_vec();

        // ASSERT
        assert_eq!(data.len(), 12);
        assert_eq!(data.get(0..8), Some(1_234_u64.to_le_bytes().as_slice()));
    }
}
