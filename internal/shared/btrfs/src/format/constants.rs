// Btrfs magic numbers
pub const BTRFS_MAGIC: u64 = 0x4D5F53665248425F;
pub const BTRFS_MAGIC_TEMPORARY: u64 = 0x4D5F536652484221;

// Superblock
pub const BTRFS_SUPER_INFO_OFFSET: u64 = 65536;
pub const BTRFS_SUPER_INFO_SIZE: usize = 4096;
pub const BTRFS_SYSTEM_CHUNK_ARRAY_SIZE: usize = 2048;
pub const BTRFS_SUPER_MIRROR_MAX: u32 = 3;

// Sizes
pub const BTRFS_DEFAULT_NODESIZE: u32 = 16384;
pub const BTRFS_DEFAULT_SECTORSIZE: u32 = 4096;
pub const BTRFS_STRIPE_LEN: u64 = 65536;
pub const BTRFS_CSUM_SIZE: usize = 32;
pub const BTRFS_LABEL_SIZE: usize = 256;
pub const BTRFS_UUID_SIZE: usize = 16;
pub const BTRFS_FSID_SIZE: usize = 16;

// Tree object IDs
pub const BTRFS_ROOT_TREE_OBJECTID: u64 = 1;
pub const BTRFS_EXTENT_TREE_OBJECTID: u64 = 2;
pub const BTRFS_CHUNK_TREE_OBJECTID: u64 = 3;
pub const BTRFS_DEV_TREE_OBJECTID: u64 = 4;
pub const BTRFS_FS_TREE_OBJECTID: u64 = 5;
pub const BTRFS_ROOT_TREE_DIR_OBJECTID: u64 = 6;
pub const BTRFS_CSUM_TREE_OBJECTID: u64 = 7;
pub const BTRFS_UUID_TREE_OBJECTID: u64 = 9;
pub const BTRFS_FREE_SPACE_TREE_OBJECTID: u64 = 10;
pub const BTRFS_BLOCK_GROUP_TREE_OBJECTID: u64 = 11;
pub const BTRFS_DATA_RELOC_TREE_OBJECTID: u64 = u64::MAX - 8;
pub const BTRFS_DEV_STATS_OBJECTID: u64 = 0;

// Special object IDs
pub const BTRFS_DEV_ITEMS_OBJECTID: u64 = 1;
pub const BTRFS_FIRST_CHUNK_TREE_OBJECTID: u64 = 256;

// Inode-related constants
pub const BTRFS_FIRST_FREE_OBJECTID: u64 = 256;

// Item key types
pub const BTRFS_INODE_ITEM_KEY: u8 = 1;
pub const BTRFS_INODE_REF_KEY: u8 = 12;
pub const BTRFS_DIR_ITEM_KEY: u8 = 84;
pub const BTRFS_ROOT_ITEM_KEY: u8 = 132;
pub const BTRFS_METADATA_ITEM_KEY: u8 = 169;
pub const BTRFS_TREE_BLOCK_REF_KEY: u8 = 176;
pub const BTRFS_BLOCK_GROUP_ITEM_KEY: u8 = 192;
pub const BTRFS_FREE_SPACE_INFO_KEY: u8 = 198;
pub const BTRFS_FREE_SPACE_EXTENT_KEY: u8 = 199;
pub const BTRFS_DEV_EXTENT_KEY: u8 = 204;
pub const BTRFS_DEV_ITEM_KEY: u8 = 216;
pub const BTRFS_CHUNK_ITEM_KEY: u8 = 228;
pub const BTRFS_UUID_KEY_SUBVOL: u8 = 251;
pub const BTRFS_PERSISTENT_ITEM_KEY: u8 = 249;

// Directory item types
pub const BTRFS_FT_DIR: u8 = 2;

// Inode mode flags
pub const S_IFDIR: u32 = 0o040000;

// Block group flags
pub const BTRFS_BLOCK_GROUP_DATA: u64 = 1 << 0;
pub const BTRFS_BLOCK_GROUP_SYSTEM: u64 = 1 << 1;
pub const BTRFS_BLOCK_GROUP_METADATA: u64 = 1 << 2;
pub const BTRFS_BLOCK_GROUP_DUP: u64 = 1 << 5;

// Feature flags - Incompat
pub const BTRFS_FEATURE_INCOMPAT_MIXED_BACKREF: u64 = 1 << 0;
pub const BTRFS_FEATURE_INCOMPAT_BIG_METADATA: u64 = 1 << 5;
pub const BTRFS_FEATURE_INCOMPAT_EXTENDED_IREF: u64 = 1 << 6;
pub const BTRFS_FEATURE_INCOMPAT_SKINNY_METADATA: u64 = 1 << 8;
pub const BTRFS_FEATURE_INCOMPAT_NO_HOLES: u64 = 1 << 9;

pub const BTRFS_FEATURE_INCOMPAT_DEFAULT: u64 = BTRFS_FEATURE_INCOMPAT_MIXED_BACKREF
    | BTRFS_FEATURE_INCOMPAT_BIG_METADATA
    | BTRFS_FEATURE_INCOMPAT_EXTENDED_IREF
    | BTRFS_FEATURE_INCOMPAT_SKINNY_METADATA
    | BTRFS_FEATURE_INCOMPAT_NO_HOLES;

// Feature flags - Compat RO
pub const BTRFS_FEATURE_COMPAT_RO_FREE_SPACE_TREE: u64 = 1 << 0;
pub const BTRFS_FEATURE_COMPAT_RO_FREE_SPACE_TREE_VALID: u64 = 1 << 1;
pub const BTRFS_FEATURE_COMPAT_RO_BLOCK_GROUP_TREE: u64 = 1 << 3;

pub const BTRFS_FEATURE_COMPAT_RO_DEFAULT: u64 = BTRFS_FEATURE_COMPAT_RO_FREE_SPACE_TREE
    | BTRFS_FEATURE_COMPAT_RO_FREE_SPACE_TREE_VALID
    | BTRFS_FEATURE_COMPAT_RO_BLOCK_GROUP_TREE;

// Checksum type
pub const BTRFS_CSUM_TYPE_CRC32: u16 = 0;

// Extent flags
pub const BTRFS_EXTENT_FLAG_TREE_BLOCK: u64 = 1 << 1;

// Header flags
pub const BTRFS_HEADER_FLAG_WRITTEN: u64 = 1 << 0;

// Backref revision (stored in upper bits of header flags)
pub const BTRFS_BACKREF_REV_SHIFT: u64 = 56;
pub const BTRFS_MIXED_BACKREF_REV: u64 = 1;

// Header size (for leaf data offset calculation)
pub const BTRFS_LEAF_DATA_OFFSET: usize = 101;

// Root item flags
pub const BTRFS_INODE_ROOT_ITEM_INIT: u64 = 1 << 31;

// Superblock flags
pub const BTRFS_SUPER_FLAG_WRITTEN: u64 = 1 << 0;

/// Superblock mirror offset: 16KiB << (12 * mirror)
pub const fn btrfs_sb_offset(mirror: u32) -> u64 {
    if mirror == 0 {
        BTRFS_SUPER_INFO_OFFSET
    } else {
        16384u64 << (12 * mirror)
    }
}

/// Reserved area before filesystem data (first 1 MiB).
pub const BTRFS_MKFS_RESERVED_SIZE: u64 = 1024 * 1024;

/// Data chunk size: 8 MiB SINGLE.
pub const BTRFS_MKFS_DATA_GROUP_SIZE: u64 = 8 * 1024 * 1024;

/// System DUP chunk logical size: 8 MiB.
pub const BTRFS_MKFS_SYSTEM_DUP_SIZE: u64 = 8 * 1024 * 1024;

/// DUP metadata minimum stripe size.
pub const BTRFS_MKFS_META_DUP_MIN_STRIPE: u64 = 32 * 1024 * 1024;

/// Number of metadata tree blocks (block-group-tree enabled: 10 total).
pub const BTRFS_MKFS_TREE_BLOCK_COUNT: u64 = 10;
