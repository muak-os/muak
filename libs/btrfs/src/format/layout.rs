//! Disk layout calculations for btrfs filesystem creation.

/// Object ID of the root tree.
pub(super) const BTRFS_ROOT_TREE_OBJECTID: u64 = 1;

/// Object ID of the extent tree.
pub(super) const BTRFS_EXTENT_TREE_OBJECTID: u64 = 2;

/// Object ID of the chunk tree.
pub(super) const BTRFS_CHUNK_TREE_OBJECTID: u64 = 3;

/// Object ID of the device tree.
pub(super) const BTRFS_DEV_TREE_OBJECTID: u64 = 4;

/// Object ID of the default filesystem tree.
pub(super) const BTRFS_FS_TREE_OBJECTID: u64 = 5;

/// Object ID of the checksum tree.
pub(super) const BTRFS_CSUM_TREE_OBJECTID: u64 = 7;

/// Object ID of the UUID tree.
pub(super) const BTRFS_UUID_TREE_OBJECTID: u64 = 9;

/// Object ID of the free-space tree.
pub(super) const BTRFS_FREE_SPACE_TREE_OBJECTID: u64 = 10;

/// Object ID of the block-group tree.
pub(super) const BTRFS_BLOCK_GROUP_TREE_OBJECTID: u64 = 11;

/// Object ID of the data relocation tree.
pub(super) const BTRFS_DATA_RELOC_TREE_OBJECTID: u64 = u64::MAX - 8;

/// First regular object ID used for chunk-tree items.
pub(super) const BTRFS_FIRST_CHUNK_TREE_OBJECTID: u64 = 256;

/// Default node size used by mkfs in bytes.
pub(super) const BTRFS_DEFAULT_NODESIZE: u32 = 16_384;

/// Default node size as a 64-bit byte count.
pub(super) const BTRFS_DEFAULT_NODESIZE_U64: u64 = 16_384;

/// Default node size as a `usize` byte count.
pub(super) const BTRFS_DEFAULT_NODESIZE_USIZE: usize = 16_384;

/// Default sector size used by mkfs in bytes.
pub(super) const BTRFS_DEFAULT_SECTORSIZE: u32 = 4096;

/// Default sector size as a 64-bit byte count.
pub(super) const BTRFS_DEFAULT_SECTORSIZE_U64: u64 = 4096;

/// Stripe length used for generated chunk items.
pub(super) const BTRFS_STRIPE_LEN: u64 = 65_536;

/// Reserved area before filesystem data.
pub(super) const BTRFS_MKFS_RESERVED_SIZE: u64 = 1024 * 1024;

/// Reserved area before filesystem data as `usize`.
pub(super) const BTRFS_MKFS_RESERVED_SIZE_USIZE: usize = 1024 * 1024;

/// Data chunk size used by mkfs.
pub(super) const BTRFS_MKFS_DATA_GROUP_SIZE: u64 = 8 * 1024 * 1024;

/// Logical size of the DUP system chunk.
pub(super) const BTRFS_MKFS_SYSTEM_DUP_SIZE: u64 = 8 * 1024 * 1024;

/// Minimum stripe size for DUP metadata chunks.
const BTRFS_MKFS_META_DUP_MIN_STRIPE: u64 = 32 * 1024 * 1024;

/// Number of tree blocks written by mkfs.
const BTRFS_MKFS_TREE_BLOCK_COUNT: u64 = 10;

/// Number of metadata tree blocks written by mkfs.
pub(super) const BTRFS_MKFS_METADATA_TREE_BLOCK_COUNT: u64 = 9;

const META_BLOCK_COUNT: usize = 9;
const DATA_LOGICAL_OFFSET: u64 = BTRFS_MKFS_RESERVED_SIZE
    .saturating_add(BTRFS_MKFS_TEMP_SYSTEM_SIZE)
    .saturating_add(BTRFS_MKFS_TEMP_META_SIZE);
const SYSTEM_LOGICAL_OFFSET: u64 = DATA_LOGICAL_OFFSET.saturating_add(BTRFS_MKFS_DATA_GROUP_SIZE);
const META_LOGICAL_OFFSET: u64 = SYSTEM_LOGICAL_OFFSET.saturating_add(BTRFS_MKFS_SYSTEM_DUP_SIZE);
const TOTAL_TREE_BYTES_USED: u64 =
    BTRFS_MKFS_TREE_BLOCK_COUNT.saturating_mul(BTRFS_DEFAULT_NODESIZE_U64);
const META_TREE_BYTES_USED: u64 =
    BTRFS_MKFS_METADATA_TREE_BLOCK_COUNT.saturating_mul(BTRFS_DEFAULT_NODESIZE_U64);

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
    meta_blocks: [u64; META_BLOCK_COUNT],
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
        let sys_phys_0 = data_phys.saturating_add(BTRFS_MKFS_DATA_GROUP_SIZE);
        let sys_phys_1 = sys_phys_0.saturating_add(BTRFS_MKFS_SYSTEM_DUP_SIZE);
        let meta_phys_0 = sys_phys_1.saturating_add(BTRFS_MKFS_SYSTEM_DUP_SIZE);
        let meta_phys_1 = meta_phys_0.saturating_add(meta_stripe_size);

        // Chunk tree root is at sys_logical + 16KiB (one nodesize into the system chunk).
        let chunk_tree_logical = sys_logical.saturating_add(BTRFS_DEFAULT_NODESIZE_U64);

        // 9 metadata tree blocks laid out sequentially at the start of the metadata chunk.
        let mut meta_blocks = [0_u64; META_BLOCK_COUNT];
        for (index, block) in meta_blocks.iter_mut().enumerate() {
            let index = u64::try_from(index).unwrap_or(0);
            *block = meta_logical.saturating_add(index.saturating_mul(BTRFS_DEFAULT_NODESIZE_U64));
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
    #[must_use]
    pub fn dev_bytes_used(&self) -> u64 {
        BTRFS_MKFS_DATA_GROUP_SIZE
            .saturating_add(BTRFS_MKFS_SYSTEM_DUP_SIZE.saturating_mul(2))
            .saturating_add(self.meta_stripe_size.saturating_mul(2))
    }

    /// Total logical bytes used (tree blocks only).
    #[must_use]
    pub const fn total_bytes_used() -> u64 {
        TOTAL_TREE_BYTES_USED
    }

    /// Metadata logical bytes used (9 tree blocks in metadata chunk).
    #[must_use]
    pub const fn meta_bytes_used() -> u64 {
        META_TREE_BYTES_USED
    }

    /// Minimum device size required for this layout.
    #[must_use]
    pub fn min_device_size(&self) -> u64 {
        BTRFS_MKFS_RESERVED_SIZE
            .saturating_add(BTRFS_MKFS_DATA_GROUP_SIZE)
            .saturating_add(BTRFS_MKFS_SYSTEM_DUP_SIZE.saturating_mul(2))
            .saturating_add(self.meta_stripe_size.saturating_mul(2))
    }

    /// Convert a logical address in the system DUP chunk to physical (stripe 0).
    #[must_use]
    pub fn sys_logical_to_phys(&self, logical: u64) -> u64 {
        self.sys_phys_0
            .saturating_add(logical.saturating_sub(self.sys_logical))
    }

    /// Convert a logical address in the metadata DUP chunk to physical (stripe 0).
    #[must_use]
    pub fn meta_logical_to_phys(&self, logical: u64) -> u64 {
        self.meta_phys_0
            .saturating_add(logical.saturating_sub(self.meta_logical))
    }

    /// Get the data chunk logical offset.
    #[must_use]
    pub fn data_logical(&self) -> u64 {
        self.data_logical
    }

    /// Get the data chunk physical offset.
    #[must_use]
    pub fn data_phys(&self) -> u64 {
        self.data_phys
    }

    /// Get the system chunk logical offset.
    #[must_use]
    pub fn sys_logical(&self) -> u64 {
        self.sys_logical
    }

    /// Get the system chunk physical stripe 0 offset.
    #[must_use]
    pub fn sys_phys_0(&self) -> u64 {
        self.sys_phys_0
    }

    /// Get the system chunk physical stripe 1 offset.
    #[must_use]
    pub fn sys_phys_1(&self) -> u64 {
        self.sys_phys_1
    }

    /// Get the metadata chunk logical offset.
    #[must_use]
    pub fn meta_logical(&self) -> u64 {
        self.meta_logical
    }

    /// Get the metadata stripe size.
    #[must_use]
    pub fn meta_stripe_size(&self) -> u64 {
        self.meta_stripe_size
    }

    /// Get the metadata chunk physical stripe 0 offset.
    #[must_use]
    pub fn meta_phys_0(&self) -> u64 {
        self.meta_phys_0
    }

    /// Get the metadata chunk physical stripe 1 offset.
    #[must_use]
    pub fn meta_phys_1(&self) -> u64 {
        self.meta_phys_1
    }

    /// Get the chunk tree logical offset.
    #[must_use]
    pub fn chunk_tree_logical(&self) -> u64 {
        self.chunk_tree_logical
    }

    /// Get a specific metadata block offset by index.
    #[must_use]
    pub fn meta_block(&self, index: usize) -> u64 {
        self.meta_blocks.get(index).copied().unwrap_or(0)
    }
}

/// Tree block logical offsets with their owner tree IDs (all 10 blocks including chunk tree).
#[must_use]
pub fn all_tree_blocks(layout: &DiskLayout) -> [(u64, u64); 10] {
    [
        (layout.chunk_tree_logical(), BTRFS_CHUNK_TREE_OBJECTID),
        (
            layout.meta_block(BLK_BLOCK_GROUP),
            BTRFS_BLOCK_GROUP_TREE_OBJECTID,
        ),
        (layout.meta_block(BLK_DEV), BTRFS_DEV_TREE_OBJECTID),
        (layout.meta_block(BLK_FS), BTRFS_FS_TREE_OBJECTID),
        (layout.meta_block(BLK_UUID), BTRFS_UUID_TREE_OBJECTID),
        (layout.meta_block(BLK_CSUM), BTRFS_CSUM_TREE_OBJECTID),
        (
            layout.meta_block(BLK_DATA_RELOC),
            BTRFS_DATA_RELOC_TREE_OBJECTID,
        ),
        (
            layout.meta_block(BLK_FREE_SPACE),
            BTRFS_FREE_SPACE_TREE_OBJECTID,
        ),
        (layout.meta_block(BLK_EXTENT), BTRFS_EXTENT_TREE_OBJECTID),
        (layout.meta_block(BLK_ROOT), BTRFS_ROOT_TREE_OBJECTID),
    ]
}

/// Compute final chunk layout offsets matching btrfs-progs sequential allocation.
#[must_use]
pub fn compute_data_logical_offset() -> u64 {
    DATA_LOGICAL_OFFSET
}

/// Compute system logical offset.
#[must_use]
pub fn compute_system_logical_offset() -> u64 {
    SYSTEM_LOGICAL_OFFSET
}

/// Compute metadata logical offset.
#[must_use]
pub fn compute_meta_logical_offset() -> u64 {
    META_LOGICAL_OFFSET
}

/// Compute DUP metadata stripe size from device size.
///
/// Matches btrfs-progs `init_alloc_chunk_ctl_policy_regular` +
/// `decide_stripe_size_regular` for METADATA|DUP.
#[must_use]
pub fn compute_dup_meta_stripe_size(device_size: u64) -> u64 {
    let initial = if device_size > 50_u64.saturating_mul(1024 * 1024 * 1024) {
        1024 * 1024 * 1024
    } else {
        256 * 1024 * 1024
    };

    let max_chunk_size = device_size.checked_div(10).unwrap_or(0).min(initial);

    let stripe = if initial > max_chunk_size {
        round_down(max_chunk_size.checked_div(2).unwrap_or(0), BTRFS_STRIPE_LEN)
    } else {
        initial
    };

    let stripe = stripe.max(BTRFS_MKFS_META_DUP_MIN_STRIPE);
    round_down(stripe, BTRFS_STRIPE_LEN)
}

fn round_down(value: u64, align: u64) -> u64 {
    value & !align.saturating_sub(1)
}

// Temporary chunk sizes used for layout calculation
const BTRFS_MKFS_TEMP_SYSTEM_SIZE: u64 = 4 * 1024 * 1024;
const BTRFS_MKFS_TEMP_META_SIZE: u64 = 8 * 1024 * 1024;

#[cfg(test)]
mod tests {
    use super::{
        BLK_BLOCK_GROUP, BLK_DEV, BLK_EXTENT, BLK_FS, BLK_ROOT, BTRFS_BLOCK_GROUP_TREE_OBJECTID,
        BTRFS_CHUNK_TREE_OBJECTID, BTRFS_CSUM_TREE_OBJECTID, BTRFS_DATA_RELOC_TREE_OBJECTID,
        BTRFS_DEV_TREE_OBJECTID, BTRFS_EXTENT_TREE_OBJECTID, BTRFS_FREE_SPACE_TREE_OBJECTID,
        BTRFS_FS_TREE_OBJECTID, BTRFS_MKFS_RESERVED_SIZE, BTRFS_MKFS_SYSTEM_DUP_SIZE,
        BTRFS_ROOT_TREE_OBJECTID, BTRFS_UUID_TREE_OBJECTID, DiskLayout, all_tree_blocks,
        compute_data_logical_offset, compute_dup_meta_stripe_size, compute_meta_logical_offset,
        compute_system_logical_offset,
    };

    const DEVICE_5G: u64 = 5_368_709_120;

    #[test]
    fn five_gib_device_uses_256mib_metadata_stripes() {
        // ARRANGE
        let device_size = DEVICE_5G;

        // ACT
        let stripe = compute_dup_meta_stripe_size(device_size);

        // ASSERT
        assert_eq!(stripe, 268_435_456);
    }

    #[test]
    fn large_devices_cap_metadata_stripes_at_1gib() {
        // ARRANGE
        let device_size = 107_374_182_400_u64;

        // ACT
        let stripe = compute_dup_meta_stripe_size(device_size);

        // ASSERT
        assert_eq!(stripe, 1_073_741_824);
    }

    #[test]
    fn small_devices_clamp_metadata_stripes_to_minimum() {
        // ARRANGE
        let hundred_mib = 104_857_600_u64;
        let one_mib = 1_048_576_u64;

        // ACT
        let stripe_small = compute_dup_meta_stripe_size(hundred_mib);
        let stripe_tiny = compute_dup_meta_stripe_size(one_mib);

        // ASSERT
        assert_eq!(stripe_small, 33_554_432);
        assert_eq!(stripe_tiny, 33_554_432);
    }

    #[test]
    fn chunk_offsets_follow_reserved_temp_then_final_order() {
        // ASSERT
        assert_eq!(compute_data_logical_offset(), 13_631_488);
        assert_eq!(compute_system_logical_offset(), 22_020_096);
        assert_eq!(compute_meta_logical_offset(), 30_408_704);
    }

    #[test]
    fn layout_offsets_match_sequential_btrfs_progs_allocation() {
        // ARRANGE
        let layout = DiskLayout::new(DEVICE_5G);

        // ACT + ASSERT
        assert_eq!(layout.data_logical(), 13_631_488);
        assert_eq!(layout.data_phys(), 13_631_488);
        assert_eq!(layout.sys_logical(), 22_020_096);
        assert_eq!(layout.sys_phys_0(), 22_020_096);
        assert_eq!(layout.sys_phys_1(), 30_408_704);
        assert_eq!(layout.meta_logical(), 30_408_704);
        assert_eq!(layout.meta_stripe_size(), 268_435_456);
        assert_eq!(layout.meta_phys_0(), 38_797_312);
        assert_eq!(layout.meta_phys_1(), 307_232_768);
        assert_eq!(layout.chunk_tree_logical(), 22_036_480);
    }

    #[test]
    fn meta_blocks_are_sequential_nodesize_steps() {
        // ARRANGE
        let layout = DiskLayout::new(DEVICE_5G);

        // ACT + ASSERT
        assert_eq!(layout.meta_block(BLK_BLOCK_GROUP), 30_408_704);
        assert_eq!(layout.meta_block(BLK_DEV), 30_425_088);
        assert_eq!(layout.meta_block(BLK_FS), 30_441_472);
        assert_eq!(layout.meta_block(BLK_EXTENT), 30_523_392);
        assert_eq!(layout.meta_block(BLK_ROOT), 30_539_776);
        assert_eq!(layout.meta_block(9), 0);
    }

    #[test]
    fn all_tree_blocks_pair_offsets_with_owner_objectids() {
        // ARRANGE
        let layout = DiskLayout::new(DEVICE_5G);

        // ACT
        let blocks = all_tree_blocks(&layout);

        // ASSERT
        assert_eq!(blocks.len(), 10);
        assert_eq!(
            blocks,
            [
                (22_036_480, BTRFS_CHUNK_TREE_OBJECTID),
                (30_408_704, BTRFS_BLOCK_GROUP_TREE_OBJECTID),
                (30_425_088, BTRFS_DEV_TREE_OBJECTID),
                (30_441_472, BTRFS_FS_TREE_OBJECTID),
                (30_457_856, BTRFS_UUID_TREE_OBJECTID),
                (30_474_240, BTRFS_CSUM_TREE_OBJECTID),
                (30_490_624, BTRFS_DATA_RELOC_TREE_OBJECTID),
                (30_507_008, BTRFS_FREE_SPACE_TREE_OBJECTID),
                (30_523_392, BTRFS_EXTENT_TREE_OBJECTID),
                (30_539_776, BTRFS_ROOT_TREE_OBJECTID),
            ]
        );
    }

    #[test]
    fn logical_to_phys_maps_into_first_stripe() {
        // ARRANGE
        let layout = DiskLayout::new(DEVICE_5G);

        // ACT + ASSERT
        assert_eq!(
            layout.sys_logical_to_phys(layout.sys_logical()),
            layout.sys_phys_0()
        );
        assert_eq!(
            layout.sys_logical_to_phys(layout.sys_logical().saturating_add(4_096)),
            layout.sys_phys_0().saturating_add(4_096)
        );
        assert_eq!(
            layout.meta_logical_to_phys(layout.meta_logical()),
            layout.meta_phys_0()
        );
    }

    #[test]
    fn byte_accounting_matches_documented_allocation() {
        // ARRANGE
        let layout = DiskLayout::new(DEVICE_5G);

        // ACT + ASSERT
        assert_eq!(layout.dev_bytes_used(), 562_036_736);
        assert_eq!(
            layout.min_device_size(),
            BTRFS_MKFS_RESERVED_SIZE
                .saturating_add(8_388_608)
                .saturating_add(BTRFS_MKFS_SYSTEM_DUP_SIZE.saturating_mul(2))
                .saturating_add(268_435_456_u64.saturating_mul(2))
        );
        assert_eq!(DiskLayout::total_bytes_used(), 163_840);
        assert_eq!(DiskLayout::meta_bytes_used(), 147_456);
    }
}
