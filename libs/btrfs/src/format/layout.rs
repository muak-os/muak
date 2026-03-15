//! Disk layout calculations for btrfs filesystem creation.

use super::constants::*;

/// Pre-computed disk layout matching btrfs-progs sequential chunk allocation.
///
/// Physical layout for a 5GiB device:
/// ```text
/// [0, 1MiB)         Reserved (superblock at 64K)
/// [1MiB, 13MiB)     FREE (temp chunks removed by cleanup_temp_chunks)
/// [13MiB, 21MiB)    Data SINGLE (8MiB)
/// [21MiB, 29MiB)    System DUP stripe 0 (8MiB)
/// [29MiB, 37MiB)    System DUP stripe 1 (8MiB)
/// [37MiB, 293MiB)   Metadata DUP stripe 0 (256MiB)
/// [293MiB, 549MiB)  Metadata DUP stripe 1 (256MiB)
/// ```
#[derive(Debug, Clone)]
pub struct DiskLayout {
    data_logical: u64,
    data_phys: u64,

    sys_logical: u64,
    sys_phys_0: u64,
    sys_phys_1: u64,

    meta_logical: u64,
    meta_stripe_size: u64,
    meta_phys_0: u64,
    meta_phys_1: u64,

    /// Logical offset of the chunk tree root (inside system DUP chunk).
    chunk_tree_logical: u64,

    /// Logical offsets for the metadata tree blocks (inside metadata DUP chunk).
    meta_blocks: [u64; 9],
}

/// Metadata tree block indices within `meta_blocks`.
pub const BLK_BLOCK_GROUP: usize = 0;
pub const BLK_DEV: usize = 1;
pub const BLK_FS: usize = 2;
pub const BLK_UUID: usize = 3;
pub const BLK_CSUM: usize = 4;
pub const BLK_DATA_RELOC: usize = 5;
pub const BLK_FREE_SPACE: usize = 6;
pub const BLK_EXTENT: usize = 7;
pub const BLK_ROOT: usize = 8;

impl DiskLayout {
    /// Create a new disk layout for the given device size.
    pub fn new(device_size: u64) -> Self {
        let meta_stripe_size = compute_dup_meta_stripe_size(device_size);

        let data_logical = compute_data_logical_offset();
        let sys_logical = compute_system_logical_offset();
        let meta_logical = compute_meta_logical_offset();

        // Physical allocation follows logical order (no gaps from temp chunks on disk)
        let data_phys = data_logical;
        let sys_phys_0 = data_phys + BTRFS_MKFS_DATA_GROUP_SIZE;
        let sys_phys_1 = sys_phys_0 + BTRFS_MKFS_SYSTEM_DUP_SIZE;
        let meta_phys_0 = sys_phys_1 + BTRFS_MKFS_SYSTEM_DUP_SIZE;
        let meta_phys_1 = meta_phys_0 + meta_stripe_size;

        // Chunk tree root is at sys_logical + 16KiB (one nodesize into the system chunk).
        let chunk_tree_logical = sys_logical + BTRFS_DEFAULT_NODESIZE as u64;

        // 9 metadata tree blocks laid out sequentially at the start of the metadata chunk.
        let mut meta_blocks = [0u64; 9];
        for (i, block) in meta_blocks.iter_mut().enumerate() {
            *block = meta_logical + (i as u64) * BTRFS_DEFAULT_NODESIZE as u64;
        }

        Self {
            data_logical,
            data_phys,
            sys_logical,
            sys_phys_0,
            sys_phys_1,
            meta_logical,
            meta_stripe_size,
            meta_phys_0,
            meta_phys_1,
            chunk_tree_logical,
            meta_blocks,
        }
    }

    /// Sum of all physical device extents.
    pub fn dev_bytes_used(&self) -> u64 {
        BTRFS_MKFS_DATA_GROUP_SIZE + 2 * BTRFS_MKFS_SYSTEM_DUP_SIZE + 2 * self.meta_stripe_size
    }

    /// Total logical bytes used (tree blocks only).
    pub fn total_bytes_used(&self) -> u64 {
        BTRFS_MKFS_TREE_BLOCK_COUNT * BTRFS_DEFAULT_NODESIZE as u64
    }

    /// Metadata logical bytes used (9 tree blocks in metadata chunk).
    pub fn meta_bytes_used(&self) -> u64 {
        9 * BTRFS_DEFAULT_NODESIZE as u64
    }

    /// Minimum device size required for this layout.
    pub fn min_device_size(&self) -> u64 {
        BTRFS_MKFS_RESERVED_SIZE
            + BTRFS_MKFS_DATA_GROUP_SIZE
            + 2 * BTRFS_MKFS_SYSTEM_DUP_SIZE
            + 2 * self.meta_stripe_size
    }

    /// Convert a logical address in the system DUP chunk to physical (stripe 0).
    pub fn sys_logical_to_phys(&self, logical: u64) -> u64 {
        self.sys_phys_0 + (logical - self.sys_logical)
    }

    /// Convert a logical address in the metadata DUP chunk to physical (stripe 0).
    pub fn meta_logical_to_phys(&self, logical: u64) -> u64 {
        self.meta_phys_0 + (logical - self.meta_logical)
    }

    /// Get the data chunk logical offset.
    pub fn data_logical(&self) -> u64 {
        self.data_logical
    }

    /// Get the data chunk physical offset.
    pub fn data_phys(&self) -> u64 {
        self.data_phys
    }

    /// Get the system chunk logical offset.
    pub fn sys_logical(&self) -> u64 {
        self.sys_logical
    }

    /// Get the system chunk physical stripe 0 offset.
    pub fn sys_phys_0(&self) -> u64 {
        self.sys_phys_0
    }

    /// Get the system chunk physical stripe 1 offset.
    pub fn sys_phys_1(&self) -> u64 {
        self.sys_phys_1
    }

    /// Get the metadata chunk logical offset.
    pub fn meta_logical(&self) -> u64 {
        self.meta_logical
    }

    /// Get the metadata stripe size.
    pub fn meta_stripe_size(&self) -> u64 {
        self.meta_stripe_size
    }

    /// Get the metadata chunk physical stripe 0 offset.
    pub fn meta_phys_0(&self) -> u64 {
        self.meta_phys_0
    }

    /// Get the metadata chunk physical stripe 1 offset.
    pub fn meta_phys_1(&self) -> u64 {
        self.meta_phys_1
    }

    /// Get the chunk tree logical offset.
    pub fn chunk_tree_logical(&self) -> u64 {
        self.chunk_tree_logical
    }

    /// Get a specific metadata block offset by index.
    pub fn meta_block(&self, index: usize) -> u64 {
        self.meta_blocks[index]
    }
}

/// Tree block logical offsets with their owner tree IDs (all 10 blocks including chunk tree).
pub fn all_tree_blocks(layout: &DiskLayout) -> [(u64, u64); 10] {
    [
        (layout.chunk_tree_logical(), BTRFS_CHUNK_TREE_OBJECTID),
        (
            layout.meta_blocks[BLK_BLOCK_GROUP],
            BTRFS_BLOCK_GROUP_TREE_OBJECTID,
        ),
        (layout.meta_blocks[BLK_DEV], BTRFS_DEV_TREE_OBJECTID),
        (layout.meta_blocks[BLK_FS], BTRFS_FS_TREE_OBJECTID),
        (layout.meta_blocks[BLK_UUID], BTRFS_UUID_TREE_OBJECTID),
        (layout.meta_blocks[BLK_CSUM], BTRFS_CSUM_TREE_OBJECTID),
        (
            layout.meta_blocks[BLK_DATA_RELOC],
            BTRFS_DATA_RELOC_TREE_OBJECTID,
        ),
        (
            layout.meta_blocks[BLK_FREE_SPACE],
            BTRFS_FREE_SPACE_TREE_OBJECTID,
        ),
        (layout.meta_blocks[BLK_EXTENT], BTRFS_EXTENT_TREE_OBJECTID),
        (layout.meta_blocks[BLK_ROOT], BTRFS_ROOT_TREE_OBJECTID),
    ]
}

/// Compute final chunk layout offsets matching btrfs-progs sequential allocation.
pub fn compute_data_logical_offset() -> u64 {
    BTRFS_MKFS_RESERVED_SIZE + BTRFS_MKFS_TEMP_SYSTEM_SIZE + BTRFS_MKFS_TEMP_META_SIZE
}

/// Compute system logical offset.
pub fn compute_system_logical_offset() -> u64 {
    compute_data_logical_offset() + BTRFS_MKFS_DATA_GROUP_SIZE
}

/// Compute metadata logical offset.
pub fn compute_meta_logical_offset() -> u64 {
    compute_system_logical_offset() + BTRFS_MKFS_SYSTEM_DUP_SIZE
}

/// Compute DUP metadata stripe size from device size.
///
/// Matches btrfs-progs `init_alloc_chunk_ctl_policy_regular` +
/// `decide_stripe_size_regular` for METADATA|DUP.
pub fn compute_dup_meta_stripe_size(device_size: u64) -> u64 {
    let initial = if device_size > 50 * 1024 * 1024 * 1024 {
        1024 * 1024 * 1024 // 1 GiB
    } else {
        256 * 1024 * 1024 // 256 MiB
    };

    let max_chunk_size = (device_size / 10).min(initial);

    let stripe = if initial > max_chunk_size {
        round_down(max_chunk_size / 2, BTRFS_STRIPE_LEN)
    } else {
        initial
    };

    let stripe = stripe.max(BTRFS_MKFS_META_DUP_MIN_STRIPE);
    round_down(stripe, BTRFS_STRIPE_LEN)
}

fn round_down(value: u64, align: u64) -> u64 {
    value & !(align - 1)
}

// Temporary chunk sizes used for layout calculation
const BTRFS_MKFS_TEMP_SYSTEM_SIZE: u64 = 4 * 1024 * 1024;
const BTRFS_MKFS_TEMP_META_SIZE: u64 = 8 * 1024 * 1024;
