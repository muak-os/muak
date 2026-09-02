//! Tree builders for btrfs filesystem creation.
//!
//! Each tree type has its own builder that encapsulates the logic for
//! creating that specific tree structure.

use uuid::Uuid;

use super::accessors::{write_u32, write_u64, write_uuid};
use super::builders::{
    InodeItemBuilder, RootItemBuilder, build_dev_extent, build_dir_item, build_inode_ref,
};
use super::chunk::{
    BTRFS_BLOCK_GROUP_DATA, BTRFS_BLOCK_GROUP_DUP, BTRFS_BLOCK_GROUP_METADATA,
    BTRFS_BLOCK_GROUP_SYSTEM, ChunkBuilder,
};
use super::layout::{
    self, BTRFS_BLOCK_GROUP_TREE_OBJECTID, BTRFS_CHUNK_TREE_OBJECTID, BTRFS_CSUM_TREE_OBJECTID,
    BTRFS_DATA_RELOC_TREE_OBJECTID, BTRFS_DEFAULT_NODESIZE_U64, BTRFS_DEFAULT_SECTORSIZE,
    BTRFS_DEV_TREE_OBJECTID, BTRFS_EXTENT_TREE_OBJECTID, BTRFS_FIRST_CHUNK_TREE_OBJECTID,
    BTRFS_FREE_SPACE_TREE_OBJECTID, BTRFS_FS_TREE_OBJECTID, BTRFS_MKFS_DATA_GROUP_SIZE,
    BTRFS_MKFS_METADATA_TREE_BLOCK_COUNT, BTRFS_MKFS_SYSTEM_DUP_SIZE, BTRFS_ROOT_TREE_OBJECTID,
    BTRFS_UUID_TREE_OBJECTID, DiskLayout,
};
use super::node::LeafBuilder;
use super::structures::{
    AsBytes as _, BtrfsBlockGroupItem, BtrfsDevItem, BtrfsDevStatsItem, BtrfsExtentItem,
    BtrfsFreeSpaceInfo, BtrfsItem,
};
use crate::error::Result;

/// Object ID for device items in the chunk tree.
const BTRFS_DEV_ITEMS_OBJECTID: u64 = 1;

/// First object ID available for filesystem tree entries.
const BTRFS_FIRST_FREE_OBJECTID: u64 = 256;

/// Object ID of the root-tree directory.
pub(super) const BTRFS_ROOT_TREE_DIR_OBJECTID: u64 = 6;

/// Object ID used by device stats items.
const BTRFS_DEV_STATS_OBJECTID: u64 = 0;

/// Inode item key type.
const BTRFS_INODE_ITEM_KEY: u8 = 1;

/// Inode reference key type.
const BTRFS_INODE_REF_KEY: u8 = 12;

/// Directory item key type.
const BTRFS_DIR_ITEM_KEY: u8 = 84;

/// Root item key type.
pub(super) const BTRFS_ROOT_ITEM_KEY: u8 = 132;

/// Metadata item key type.
const BTRFS_METADATA_ITEM_KEY: u8 = 169;

/// Tree block reference key type.
const BTRFS_TREE_BLOCK_REF_KEY: u8 = 176;

/// Block-group item key type.
const BTRFS_BLOCK_GROUP_ITEM_KEY: u8 = 192;

/// Free-space info key type.
const BTRFS_FREE_SPACE_INFO_KEY: u8 = 198;

/// Free-space extent key type.
const BTRFS_FREE_SPACE_EXTENT_KEY: u8 = 199;

/// Device extent key type.
const BTRFS_DEV_EXTENT_KEY: u8 = 204;

/// Device item key type.
const BTRFS_DEV_ITEM_KEY: u8 = 216;

/// Chunk item key type.
pub(super) const BTRFS_CHUNK_ITEM_KEY: u8 = 228;

/// Persistent item key type.
const BTRFS_PERSISTENT_ITEM_KEY: u8 = 249;

/// UUID tree subvolume key type.
const BTRFS_UUID_KEY_SUBVOL: u8 = 251;

/// Extent item flag for tree blocks.
const BTRFS_EXTENT_FLAG_TREE_BLOCK: u64 = 1 << 1;

/// Root item inode initialization flag.
const BTRFS_INODE_ROOT_ITEM_INIT: u64 = 1 << 31;

/// Context for building trees.
#[derive(Debug)]
pub struct TreeContext<'a> {
    pub layout: &'a DiskLayout,
    pub fsid: &'a Uuid,
    pub chunk_uuid: &'a Uuid,
    pub dev_uuid: &'a Uuid,
    pub fs_uuid: &'a Uuid,
    pub generation: u64,
    pub device_size: u64,
}

impl<'a> TreeContext<'a> {
    /// Create a new tree context.
    #[must_use]
    pub fn new(
        layout: &'a DiskLayout,
        fsid: &'a Uuid,
        chunk_uuid: &'a Uuid,
        dev_uuid: &'a Uuid,
        fs_uuid: &'a Uuid,
        generation: u64,
        device_size: u64,
    ) -> Self {
        Self {
            layout,
            fsid,
            chunk_uuid,
            dev_uuid,
            fs_uuid,
            generation,
            device_size,
        }
    }
}

/// Result of building a tree.
#[derive(Debug)]
pub struct TreeResult {
    pub logical_offset: u64,
    pub owner: u64,
    pub items: Vec<BtrfsItem>,
    pub data: Vec<u8>,
}

impl TreeResult {
    /// Create a new tree result.
    #[must_use]
    pub fn new(logical_offset: u64, owner: u64, items: Vec<BtrfsItem>, data: Vec<u8>) -> Self {
        Self {
            logical_offset,
            owner,
            items,
            data,
        }
    }

    /// Create an empty tree result.
    #[must_use]
    pub fn empty(logical_offset: u64, owner: u64) -> Self {
        Self {
            logical_offset,
            owner,
            items: Vec::new(),
            data: Vec::new(),
        }
    }
}

/// Trait for tree builders.
pub trait TreeBuilder {
    /// Build the tree and return the result.
    fn build(&self, ctx: &TreeContext) -> Result<TreeResult>;
}

// Chunk tree builder
pub struct ChunkTreeBuilder;

impl ChunkTreeBuilder {
    /// Create a new chunk tree builder.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for ChunkTreeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeBuilder for ChunkTreeBuilder {
    fn build(&self, ctx: &TreeContext) -> Result<TreeResult> {
        let mut builder = LeafBuilder::new();

        // Dev item
        let dev_item = build_dev_item(ctx);
        builder.add_item(
            BTRFS_DEV_ITEMS_OBJECTID,
            BTRFS_DEV_ITEM_KEY,
            1,
            dev_item.to_vec(),
        );

        // Data chunk (SINGLE)
        let data_chunk = ChunkBuilder::single(
            BTRFS_MKFS_DATA_GROUP_SIZE,
            BTRFS_BLOCK_GROUP_DATA,
            ctx.layout.data_phys(),
        )
        .build(ctx.dev_uuid)?;
        builder.add_item(
            BTRFS_FIRST_CHUNK_TREE_OBJECTID,
            BTRFS_CHUNK_ITEM_KEY,
            ctx.layout.data_logical(),
            data_chunk,
        );

        // System chunk (DUP)
        let sys_chunk = ChunkBuilder::dup(
            BTRFS_MKFS_SYSTEM_DUP_SIZE,
            BTRFS_BLOCK_GROUP_SYSTEM,
            ctx.layout.sys_phys_0(),
            ctx.layout.sys_phys_1(),
        )
        .build(ctx.dev_uuid)?;
        builder.add_item(
            BTRFS_FIRST_CHUNK_TREE_OBJECTID,
            BTRFS_CHUNK_ITEM_KEY,
            ctx.layout.sys_logical(),
            sys_chunk.clone(),
        );

        // Metadata chunk (DUP)
        let meta_chunk = ChunkBuilder::dup(
            ctx.layout.meta_stripe_size(),
            BTRFS_BLOCK_GROUP_METADATA,
            ctx.layout.meta_phys_0(),
            ctx.layout.meta_phys_1(),
        )
        .build(ctx.dev_uuid)?;
        builder.add_item(
            BTRFS_FIRST_CHUNK_TREE_OBJECTID,
            BTRFS_CHUNK_ITEM_KEY,
            ctx.layout.meta_logical(),
            meta_chunk,
        );

        let (items, data) = builder.build()?;
        Ok(TreeResult::new(
            ctx.layout.chunk_tree_logical(),
            BTRFS_CHUNK_TREE_OBJECTID,
            items,
            data,
        ))
    }
}

/// Build a `BtrfsDevItem`.
fn build_dev_item(ctx: &TreeContext) -> BtrfsDevItem {
    let mut dev = BtrfsDevItem::new();
    write_u64(&mut dev.devid, 1);
    write_u64(&mut dev.total_bytes, ctx.device_size);
    write_u64(&mut dev.bytes_used, ctx.layout.dev_bytes_used());
    write_u32(&mut dev.io_align, BTRFS_DEFAULT_SECTORSIZE);
    write_u32(&mut dev.io_width, BTRFS_DEFAULT_SECTORSIZE);
    write_u32(&mut dev.sector_size, BTRFS_DEFAULT_SECTORSIZE);
    write_uuid(&mut dev.uuid, ctx.dev_uuid);
    write_uuid(&mut dev.fsid, ctx.fsid);
    dev
}

// Dev tree builder
pub struct DevTreeBuilder;

impl DevTreeBuilder {
    /// Create a new dev tree builder.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for DevTreeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeBuilder for DevTreeBuilder {
    fn build(&self, ctx: &TreeContext) -> Result<TreeResult> {
        let mut builder = LeafBuilder::new();

        // Dev stats
        let dev_stats = BtrfsDevStatsItem {
            values: [[0; 8]; 5],
        };
        builder.add_item(
            BTRFS_DEV_STATS_OBJECTID,
            BTRFS_PERSISTENT_ITEM_KEY,
            1,
            dev_stats.to_vec(),
        );

        // 5 dev extents: data, sys stripe 0, sys stripe 1, meta stripe 0, meta stripe 1
        let dev_extents = [
            (
                ctx.layout.data_phys(),
                ctx.layout.data_logical(),
                BTRFS_MKFS_DATA_GROUP_SIZE,
            ),
            (
                ctx.layout.sys_phys_0(),
                ctx.layout.sys_logical(),
                BTRFS_MKFS_SYSTEM_DUP_SIZE,
            ),
            (
                ctx.layout.sys_phys_1(),
                ctx.layout.sys_logical(),
                BTRFS_MKFS_SYSTEM_DUP_SIZE,
            ),
            (
                ctx.layout.meta_phys_0(),
                ctx.layout.meta_logical(),
                ctx.layout.meta_stripe_size(),
            ),
            (
                ctx.layout.meta_phys_1(),
                ctx.layout.meta_logical(),
                ctx.layout.meta_stripe_size(),
            ),
        ];

        for (phys, logical, len) in dev_extents {
            let extent = build_dev_extent(logical, len, ctx.chunk_uuid);
            builder.add_item(1, BTRFS_DEV_EXTENT_KEY, phys, extent);
        }

        let (items, data) = builder.build()?;
        Ok(TreeResult::new(
            ctx.layout.meta_block(layout::BLK_DEV),
            BTRFS_DEV_TREE_OBJECTID,
            items,
            data,
        ))
    }
}

// Extent tree builder
pub struct ExtentTreeBuilder;

impl ExtentTreeBuilder {
    /// Create a new extent tree builder.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for ExtentTreeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeBuilder for ExtentTreeBuilder {
    fn build(&self, ctx: &TreeContext) -> Result<TreeResult> {
        let mut builder = LeafBuilder::new();

        let blocks = layout::all_tree_blocks(ctx.layout);
        for (blk_offset, owner) in blocks {
            add_extent_item(&mut builder, blk_offset, owner, ctx.generation);
        }

        let (items, data) = builder.build()?;
        Ok(TreeResult::new(
            ctx.layout.meta_block(layout::BLK_EXTENT),
            BTRFS_EXTENT_TREE_OBJECTID,
            items,
            data,
        ))
    }
}

/// Add an inline extent item with `TREE_BLOCK_REF`.
fn add_extent_item(builder: &mut LeafBuilder, blk_offset: u64, owner: u64, generation: u64) {
    let mut extent = BtrfsExtentItem {
        refs: [0; 8],
        generation: [0; 8],
        flags: [0; 8],
    };
    write_u64(&mut extent.refs, 1);
    write_u64(&mut extent.generation, generation);
    write_u64(&mut extent.flags, BTRFS_EXTENT_FLAG_TREE_BLOCK);

    let mut data = extent.to_vec();
    data.push(BTRFS_TREE_BLOCK_REF_KEY);
    data.extend_from_slice(&owner.to_le_bytes());

    builder.add_item(blk_offset, BTRFS_METADATA_ITEM_KEY, 0, data);
}

// Block group tree builder
pub struct BlockGroupTreeBuilder;

impl BlockGroupTreeBuilder {
    /// Create a new block group tree builder.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for BlockGroupTreeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeBuilder for BlockGroupTreeBuilder {
    fn build(&self, ctx: &TreeContext) -> Result<TreeResult> {
        let mut builder = LeafBuilder::new();

        // Data block group
        add_block_group_item(
            &mut builder,
            ctx.layout.data_logical(),
            BTRFS_MKFS_DATA_GROUP_SIZE,
            0,
            BTRFS_BLOCK_GROUP_DATA,
        );

        // System block group
        add_block_group_item(
            &mut builder,
            ctx.layout.sys_logical(),
            BTRFS_MKFS_SYSTEM_DUP_SIZE,
            BTRFS_DEFAULT_NODESIZE_U64,
            BTRFS_BLOCK_GROUP_SYSTEM | BTRFS_BLOCK_GROUP_DUP,
        );

        // Metadata block group
        add_block_group_item(
            &mut builder,
            ctx.layout.meta_logical(),
            ctx.layout.meta_stripe_size(),
            DiskLayout::meta_bytes_used(),
            BTRFS_BLOCK_GROUP_METADATA | BTRFS_BLOCK_GROUP_DUP,
        );

        let (items, data) = builder.build()?;
        Ok(TreeResult::new(
            ctx.layout.meta_block(layout::BLK_BLOCK_GROUP),
            BTRFS_BLOCK_GROUP_TREE_OBJECTID,
            items,
            data,
        ))
    }
}

/// Add a block group item.
fn add_block_group_item(
    builder: &mut LeafBuilder,
    group_offset: u64,
    group_size: u64,
    used: u64,
    flags: u64,
) {
    let mut bg = BtrfsBlockGroupItem {
        used: [0; 8],
        chunk_objectid: [0; 8],
        flags: [0; 8],
    };
    write_u64(&mut bg.used, used);
    write_u64(&mut bg.chunk_objectid, BTRFS_FIRST_CHUNK_TREE_OBJECTID);
    write_u64(&mut bg.flags, flags);
    builder.add_item(
        group_offset,
        BTRFS_BLOCK_GROUP_ITEM_KEY,
        group_size,
        bg.to_vec(),
    );
}

// Free space tree builder
pub struct FreeSpaceTreeBuilder;

impl FreeSpaceTreeBuilder {
    /// Create a new free space tree builder.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for FreeSpaceTreeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeBuilder for FreeSpaceTreeBuilder {
    fn build(&self, ctx: &TreeContext) -> Result<TreeResult> {
        let mut builder = LeafBuilder::new();
        let nodesize = BTRFS_DEFAULT_NODESIZE_U64;

        // Data block group: completely free
        add_free_space_info(
            &mut builder,
            ctx.layout.data_logical(),
            BTRFS_MKFS_DATA_GROUP_SIZE,
            1,
        );
        builder.add_item(
            ctx.layout.data_logical(),
            BTRFS_FREE_SPACE_EXTENT_KEY,
            BTRFS_MKFS_DATA_GROUP_SIZE,
            Vec::new(),
        );

        // System DUP block group: chunk tree at sys_logical + nodesize, rest free
        let chunk_tree_start = ctx.layout.chunk_tree_logical();
        let chunk_tree_end = chunk_tree_start.saturating_add(nodesize);
        let sys_end = ctx
            .layout
            .sys_logical()
            .saturating_add(BTRFS_MKFS_SYSTEM_DUP_SIZE);

        let sys_free_before = chunk_tree_start.saturating_sub(ctx.layout.sys_logical());
        let sys_free_after = sys_end.saturating_sub(chunk_tree_end);

        let sys_extent_count =
            u32::from(sys_free_before > 0).saturating_add(u32::from(sys_free_after > 0));

        add_free_space_info(
            &mut builder,
            ctx.layout.sys_logical(),
            BTRFS_MKFS_SYSTEM_DUP_SIZE,
            sys_extent_count,
        );
        if sys_free_before > 0 {
            builder.add_item(
                ctx.layout.sys_logical(),
                BTRFS_FREE_SPACE_EXTENT_KEY,
                sys_free_before,
                Vec::new(),
            );
        }
        if sys_free_after > 0 {
            builder.add_item(
                chunk_tree_end,
                BTRFS_FREE_SPACE_EXTENT_KEY,
                sys_free_after,
                Vec::new(),
            );
        }

        // Metadata DUP block group: 9 tree blocks at start, rest free
        let meta_used_end = ctx
            .layout
            .meta_logical()
            .saturating_add(BTRFS_MKFS_METADATA_TREE_BLOCK_COUNT.saturating_mul(nodesize));
        let meta_end = ctx
            .layout
            .meta_logical()
            .saturating_add(ctx.layout.meta_stripe_size());
        let meta_free = meta_end.saturating_sub(meta_used_end);

        let meta_extent_count = u32::from(meta_free > 0);
        add_free_space_info(
            &mut builder,
            ctx.layout.meta_logical(),
            ctx.layout.meta_stripe_size(),
            meta_extent_count,
        );
        if meta_free > 0 {
            builder.add_item(
                meta_used_end,
                BTRFS_FREE_SPACE_EXTENT_KEY,
                meta_free,
                Vec::new(),
            );
        }

        let (items, data) = builder.build()?;
        Ok(TreeResult::new(
            ctx.layout.meta_block(layout::BLK_FREE_SPACE),
            BTRFS_FREE_SPACE_TREE_OBJECTID,
            items,
            data,
        ))
    }
}

/// Add a free space info item.
fn add_free_space_info(
    builder: &mut LeafBuilder,
    group_offset: u64,
    group_size: u64,
    extent_count: u32,
) {
    let mut info = BtrfsFreeSpaceInfo {
        extent_count: [0; 4],
        flags: [0; 4],
    };
    write_u32(&mut info.extent_count, extent_count);
    builder.add_item(
        group_offset,
        BTRFS_FREE_SPACE_INFO_KEY,
        group_size,
        info.to_vec(),
    );
}

// FS tree builder
pub struct FsTreeBuilder {
    now: u64,
}

impl FsTreeBuilder {
    /// Create a new FS tree builder.
    #[must_use]
    pub fn new(now: u64) -> Self {
        Self { now }
    }
}

impl TreeBuilder for FsTreeBuilder {
    fn build(&self, ctx: &TreeContext) -> Result<TreeResult> {
        let mut builder = LeafBuilder::new();

        let inode = InodeItemBuilder::new()
            .generation(ctx.generation)
            .timestamps(self.now)
            .build();
        builder.add_item(
            BTRFS_FIRST_FREE_OBJECTID,
            BTRFS_INODE_ITEM_KEY,
            0,
            inode.to_vec(),
        );
        builder.add_item(
            BTRFS_FIRST_FREE_OBJECTID,
            BTRFS_INODE_REF_KEY,
            BTRFS_FIRST_FREE_OBJECTID,
            build_inode_ref(b"..")?,
        );

        let (items, data) = builder.build()?;
        Ok(TreeResult::new(
            ctx.layout.meta_block(layout::BLK_FS),
            BTRFS_FS_TREE_OBJECTID,
            items,
            data,
        ))
    }
}

// Data reloc tree builder
pub struct DataRelocTreeBuilder {
    now: u64,
}

impl DataRelocTreeBuilder {
    /// Create a new data reloc tree builder.
    #[must_use]
    pub fn new(now: u64) -> Self {
        Self { now }
    }
}

impl TreeBuilder for DataRelocTreeBuilder {
    fn build(&self, ctx: &TreeContext) -> Result<TreeResult> {
        let mut builder = LeafBuilder::new();

        let inode = InodeItemBuilder::new()
            .generation(ctx.generation)
            .timestamps(self.now)
            .build();
        builder.add_item(
            BTRFS_FIRST_FREE_OBJECTID,
            BTRFS_INODE_ITEM_KEY,
            0,
            inode.to_vec(),
        );
        builder.add_item(
            BTRFS_FIRST_FREE_OBJECTID,
            BTRFS_INODE_REF_KEY,
            BTRFS_FIRST_FREE_OBJECTID,
            build_inode_ref(b"..")?,
        );

        let (items, data) = builder.build()?;
        Ok(TreeResult::new(
            ctx.layout.meta_block(layout::BLK_DATA_RELOC),
            BTRFS_DATA_RELOC_TREE_OBJECTID,
            items,
            data,
        ))
    }
}

// UUID tree builder
pub struct UuidTreeBuilder;

impl UuidTreeBuilder {
    /// Create a new UUID tree builder.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for UuidTreeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeBuilder for UuidTreeBuilder {
    fn build(&self, ctx: &TreeContext) -> Result<TreeResult> {
        let mut builder = LeafBuilder::new();

        let uuid_bytes = ctx.fs_uuid.as_bytes();
        let mut objectid_buf = [0_u8; 8];
        let mut offset_buf = [0_u8; 8];
        if let Some(bytes) = uuid_bytes.get(..8) {
            objectid_buf.copy_from_slice(bytes);
        }
        if let Some(bytes) = uuid_bytes.get(8..16) {
            offset_buf.copy_from_slice(bytes);
        }
        let key_objectid = u64::from_le_bytes(objectid_buf);
        let key_offset = u64::from_le_bytes(offset_buf);

        builder.add_item(
            key_objectid,
            BTRFS_UUID_KEY_SUBVOL,
            key_offset,
            BTRFS_FS_TREE_OBJECTID.to_le_bytes().to_vec(),
        );

        let (items, data) = builder.build()?;
        Ok(TreeResult::new(
            ctx.layout.meta_block(layout::BLK_UUID),
            BTRFS_UUID_TREE_OBJECTID,
            items,
            data,
        ))
    }
}

// CSUM tree builder
pub struct CsumTreeBuilder;

impl CsumTreeBuilder {
    /// Create a new CSUM tree builder.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for CsumTreeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeBuilder for CsumTreeBuilder {
    fn build(&self, ctx: &TreeContext) -> Result<TreeResult> {
        Ok(TreeResult::empty(
            ctx.layout.meta_block(layout::BLK_CSUM),
            BTRFS_CSUM_TREE_OBJECTID,
        ))
    }
}

// Root tree builder
pub struct RootTreeBuilder {
    now: u64,
}

impl RootTreeBuilder {
    /// Create a new root tree builder.
    #[must_use]
    pub fn new(now: u64) -> Self {
        Self { now }
    }

    fn add_system_roots(builder: &mut LeafBuilder, ctx: &TreeContext) {
        add_root_item(
            builder,
            BTRFS_EXTENT_TREE_OBJECTID,
            ctx.layout.meta_block(layout::BLK_EXTENT),
        );
        add_root_item(
            builder,
            BTRFS_DEV_TREE_OBJECTID,
            ctx.layout.meta_block(layout::BLK_DEV),
        );
        add_root_item(
            builder,
            BTRFS_CSUM_TREE_OBJECTID,
            ctx.layout.meta_block(layout::BLK_CSUM),
        );
        add_root_item(
            builder,
            BTRFS_UUID_TREE_OBJECTID,
            ctx.layout.meta_block(layout::BLK_UUID),
        );
        add_root_item(
            builder,
            BTRFS_FREE_SPACE_TREE_OBJECTID,
            ctx.layout.meta_block(layout::BLK_FREE_SPACE),
        );
        add_root_item(
            builder,
            BTRFS_BLOCK_GROUP_TREE_OBJECTID,
            ctx.layout.meta_block(layout::BLK_BLOCK_GROUP),
        );
        let ri = RootItemBuilder::new()
            .generation(ctx.generation)
            .bytenr(ctx.layout.meta_block(layout::BLK_DATA_RELOC))
            .root_dirid(BTRFS_FIRST_FREE_OBJECTID)
            .build();
        builder.add_item(
            BTRFS_DATA_RELOC_TREE_OBJECTID,
            BTRFS_ROOT_ITEM_KEY,
            0,
            ri.to_vec(),
        );
    }

    fn add_fs_root(&self, builder: &mut LeafBuilder, ctx: &TreeContext) -> Result<()> {
        builder.add_item(
            BTRFS_FS_TREE_OBJECTID,
            BTRFS_INODE_REF_KEY,
            BTRFS_ROOT_TREE_DIR_OBJECTID,
            build_inode_ref(b"default")?,
        );

        let ri = RootItemBuilder::new()
            .generation(ctx.generation)
            .bytenr(ctx.layout.meta_block(layout::BLK_FS))
            .root_dirid(BTRFS_FIRST_FREE_OBJECTID)
            .uuid(ctx.fs_uuid)
            .flags(BTRFS_INODE_ROOT_ITEM_INIT)
            .ctime(self.now)
            .otime(self.now)
            .build();
        builder.add_item(BTRFS_FS_TREE_OBJECTID, BTRFS_ROOT_ITEM_KEY, 0, ri.to_vec());
        Ok(())
    }

    fn add_root_dir(&self, builder: &mut LeafBuilder, ctx: &TreeContext) -> Result<()> {
        let dir_inode = InodeItemBuilder::new()
            .generation(ctx.generation)
            .timestamps(self.now)
            .build();
        builder.add_item(
            BTRFS_ROOT_TREE_DIR_OBJECTID,
            BTRFS_INODE_ITEM_KEY,
            0,
            dir_inode.to_vec(),
        );

        builder.add_item(
            BTRFS_ROOT_TREE_DIR_OBJECTID,
            BTRFS_INODE_REF_KEY,
            BTRFS_ROOT_TREE_DIR_OBJECTID,
            build_inode_ref(b"..")?,
        );

        let name = b"default";
        builder.add_item(
            BTRFS_ROOT_TREE_DIR_OBJECTID,
            BTRFS_DIR_ITEM_KEY,
            super::checksum::btrfs_name_hash(name),
            build_dir_item(
                BTRFS_FS_TREE_OBJECTID,
                BTRFS_ROOT_ITEM_KEY,
                name,
                ctx.generation,
            )?,
        );
        Ok(())
    }
}

fn add_root_item(builder: &mut LeafBuilder, objectid: u64, bytenr: u64) {
    let ri = RootItemBuilder::new().bytenr(bytenr).build();
    builder.add_item(objectid, BTRFS_ROOT_ITEM_KEY, 0, ri.to_vec());
}

impl TreeBuilder for RootTreeBuilder {
    fn build(&self, ctx: &TreeContext) -> Result<TreeResult> {
        let mut builder = LeafBuilder::new();

        Self::add_system_roots(&mut builder, ctx);
        self.add_fs_root(&mut builder, ctx)?;
        self.add_root_dir(&mut builder, ctx)?;

        let (items, data) = builder.build()?;
        Ok(TreeResult::new(
            ctx.layout.meta_block(layout::BLK_ROOT),
            BTRFS_ROOT_TREE_OBJECTID,
            items,
            data,
        ))
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::super::chunk::{
        BTRFS_BLOCK_GROUP_DATA, BTRFS_BLOCK_GROUP_DUP, BTRFS_BLOCK_GROUP_METADATA,
        BTRFS_BLOCK_GROUP_SYSTEM,
    };
    use super::super::layout::{
        BLK_BLOCK_GROUP, BLK_CSUM, BLK_DATA_RELOC, BLK_DEV, BLK_EXTENT, BLK_FREE_SPACE, BLK_FS,
        BLK_ROOT, BLK_UUID, BTRFS_BLOCK_GROUP_TREE_OBJECTID, BTRFS_CHUNK_TREE_OBJECTID,
        BTRFS_CSUM_TREE_OBJECTID, BTRFS_DATA_RELOC_TREE_OBJECTID, BTRFS_DEFAULT_NODESIZE_U64,
        BTRFS_DEV_TREE_OBJECTID, BTRFS_EXTENT_TREE_OBJECTID, BTRFS_FIRST_CHUNK_TREE_OBJECTID,
        BTRFS_FREE_SPACE_TREE_OBJECTID, BTRFS_FS_TREE_OBJECTID, BTRFS_MKFS_DATA_GROUP_SIZE,
        BTRFS_MKFS_SYSTEM_DUP_SIZE, BTRFS_ROOT_TREE_OBJECTID, BTRFS_UUID_TREE_OBJECTID,
        all_tree_blocks,
    };
    use super::*;

    const DEVICE_5G: u64 = 5_368_709_120;
    const META_STRIPE: u64 = 268_435_456;
    const META_BYTES_USED: u64 = 147_456;
    const FS_UUID_OBJECTID: u64 = 0x0807_0605_0403_0201;
    const FS_UUID_OFFSET: u64 = 0x100F_0E0D_0C0B_0A09;

    struct Fixture {
        layout: DiskLayout,
        fsid: Uuid,
        chunk_uuid: Uuid,
        dev_uuid: Uuid,
        fs_uuid: Uuid,
    }

    fn fixture() -> Fixture {
        Fixture {
            layout: DiskLayout::new(DEVICE_5G),
            fsid: Uuid::from_u128(0x0A01),
            chunk_uuid: Uuid::from_u128(0x0A02),
            dev_uuid: Uuid::from_u128(0x0A03),
            fs_uuid: Uuid::from_u128(0x0102_0304_0506_0708_090A_0B0C_0D0E_0F10),
        }
    }

    impl Fixture {
        fn context(&self) -> TreeContext<'_> {
            TreeContext::new(
                &self.layout,
                &self.fsid,
                &self.chunk_uuid,
                &self.dev_uuid,
                &self.fs_uuid,
                7,
                DEVICE_5G,
            )
        }
    }

    fn le64(data: &[u8], at: usize) -> u64 {
        u64::from_le_bytes(
            data.get(at..at.saturating_add(8))
                .unwrap()
                .try_into()
                .unwrap(),
        )
    }

    fn item_key(item: &BtrfsItem) -> (u64, u8, u64) {
        (
            u64::from_le_bytes(item.key.objectid),
            item.key.type_,
            u64::from_le_bytes(item.key.offset),
        )
    }

    fn item_size(item: &BtrfsItem) -> usize {
        usize::try_from(u32::from_le_bytes(item.size)).unwrap()
    }

    fn keys_of(result: &TreeResult) -> Vec<(u64, u8, u64)> {
        result.items.iter().map(item_key).collect()
    }

    /// Item payloads are stacked from the end of the leaf data backwards.
    fn item_payload(result: &TreeResult, index: usize) -> &[u8] {
        let start: usize = result
            .items
            .iter()
            .skip(index.saturating_add(1))
            .map(item_size)
            .sum();
        let end = start.saturating_add(item_size(result.items.get(index).unwrap()));
        result.data.get(start..end).unwrap_or(&[])
    }

    #[test]
    fn chunk_tree_contains_device_item_and_three_chunks() {
        // ARRANGE
        let fixture = fixture();
        let context = fixture.context();

        // ACT
        let result = ChunkTreeBuilder.build(&context).unwrap();

        // ASSERT
        assert_eq!(result.logical_offset, fixture.layout.chunk_tree_logical());
        assert_eq!(result.owner, BTRFS_CHUNK_TREE_OBJECTID);
        assert_eq!(
            keys_of(&result),
            vec![
                (1, BTRFS_DEV_ITEM_KEY, 1),
                (
                    BTRFS_FIRST_CHUNK_TREE_OBJECTID,
                    BTRFS_CHUNK_ITEM_KEY,
                    fixture.layout.data_logical()
                ),
                (
                    BTRFS_FIRST_CHUNK_TREE_OBJECTID,
                    BTRFS_CHUNK_ITEM_KEY,
                    fixture.layout.sys_logical()
                ),
                (
                    BTRFS_FIRST_CHUNK_TREE_OBJECTID,
                    BTRFS_CHUNK_ITEM_KEY,
                    fixture.layout.meta_logical()
                ),
            ]
        );
        assert_eq!(result.data.len(), 98 + 80 + 112 + 112);

        let dev_item = item_payload(&result, 0);
        assert_eq!(le64(dev_item, 0), 1);
        assert_eq!(le64(dev_item, 8), DEVICE_5G);
        assert_eq!(le64(dev_item, 16), fixture.layout.dev_bytes_used());
        assert_eq!(
            dev_item.get(66..82),
            Some(fixture.dev_uuid.as_bytes().as_slice())
        );
        assert_eq!(
            dev_item.get(82..98),
            Some(fixture.fsid.as_bytes().as_slice())
        );

        let data_chunk = item_payload(&result, 1);
        assert_eq!(data_chunk.len(), 80);
        assert_eq!(le64(data_chunk, 0), BTRFS_MKFS_DATA_GROUP_SIZE);
        assert_eq!(le64(data_chunk, 24), BTRFS_BLOCK_GROUP_DATA);
        assert_eq!(le64(data_chunk, 48), 1);
        assert_eq!(le64(data_chunk, 56), fixture.layout.data_phys());
    }

    #[test]
    fn dev_tree_contains_stats_and_five_dev_extents() {
        // ARRANGE
        let fixture = fixture();
        let context = fixture.context();

        // ACT
        let result = DevTreeBuilder.build(&context).unwrap();

        // ASSERT
        assert_eq!(result.owner, BTRFS_DEV_TREE_OBJECTID);
        assert_eq!(result.logical_offset, fixture.layout.meta_block(BLK_DEV));
        assert_eq!(result.items.len(), 6);
        assert_eq!(
            keys_of(&result),
            vec![
                (0, BTRFS_PERSISTENT_ITEM_KEY, 1),
                (1, BTRFS_DEV_EXTENT_KEY, fixture.layout.data_phys()),
                (1, BTRFS_DEV_EXTENT_KEY, fixture.layout.sys_phys_0()),
                (1, BTRFS_DEV_EXTENT_KEY, fixture.layout.sys_phys_1()),
                (1, BTRFS_DEV_EXTENT_KEY, fixture.layout.meta_phys_0()),
                (1, BTRFS_DEV_EXTENT_KEY, fixture.layout.meta_phys_1()),
            ]
        );

        let extent = item_payload(&result, 1);
        assert_eq!(extent.len(), 48);
        assert_eq!(le64(extent, 0), 3);
        assert_eq!(le64(extent, 8), 256);
        assert_eq!(le64(extent, 16), fixture.layout.data_logical());
        assert_eq!(le64(extent, 24), BTRFS_MKFS_DATA_GROUP_SIZE);
        assert_eq!(
            extent.get(32..48),
            Some(fixture.chunk_uuid.as_bytes().as_slice())
        );
        assert!(item_payload(&result, 0).iter().all(|&byte| byte == 0));
    }

    #[test]
    fn extent_tree_records_all_ten_tree_blocks() {
        // ARRANGE
        let fixture = fixture();
        let context = fixture.context();
        let blocks = all_tree_blocks(&fixture.layout);

        // ACT
        let result = ExtentTreeBuilder.build(&context).unwrap();

        // ASSERT
        assert_eq!(result.owner, BTRFS_EXTENT_TREE_OBJECTID);
        assert_eq!(result.logical_offset, fixture.layout.meta_block(BLK_EXTENT));
        assert_eq!(result.items.len(), 10);
        for ((index, item), block) in result.items.iter().enumerate().zip(blocks.iter()) {
            let (offset, owner) = *block;
            assert_eq!(item_key(item).0, offset);
            assert_eq!(item_key(item).1, BTRFS_METADATA_ITEM_KEY);
            let payload = item_payload(&result, index);
            assert_eq!(payload.len(), 33);
            assert_eq!(le64(payload, 0), 1);
            assert_eq!(le64(payload, 8), 7);
            assert_eq!(le64(payload, 16), BTRFS_EXTENT_FLAG_TREE_BLOCK);
            assert_eq!(payload.get(24), Some(&BTRFS_TREE_BLOCK_REF_KEY));
            assert_eq!(le64(payload, 25), owner);
        }
    }

    #[test]
    fn block_group_tree_records_three_block_groups() {
        // ARRANGE
        let fixture = fixture();
        let context = fixture.context();

        // ACT
        let result = BlockGroupTreeBuilder.build(&context).unwrap();

        // ASSERT
        assert_eq!(result.owner, BTRFS_BLOCK_GROUP_TREE_OBJECTID);
        assert_eq!(
            result.logical_offset,
            fixture.layout.meta_block(BLK_BLOCK_GROUP)
        );
        assert_eq!(result.items.len(), 3);
        assert_eq!(
            keys_of(&result),
            vec![
                (
                    fixture.layout.data_logical(),
                    BTRFS_BLOCK_GROUP_ITEM_KEY,
                    BTRFS_MKFS_DATA_GROUP_SIZE
                ),
                (
                    fixture.layout.sys_logical(),
                    BTRFS_BLOCK_GROUP_ITEM_KEY,
                    BTRFS_MKFS_SYSTEM_DUP_SIZE
                ),
                (
                    fixture.layout.meta_logical(),
                    BTRFS_BLOCK_GROUP_ITEM_KEY,
                    META_STRIPE
                ),
            ]
        );

        let data_group = item_payload(&result, 0);
        assert_eq!(le64(data_group, 0), 0);
        assert_eq!(le64(data_group, 8), BTRFS_FIRST_CHUNK_TREE_OBJECTID);
        assert_eq!(le64(data_group, 16), BTRFS_BLOCK_GROUP_DATA);

        let system_group = item_payload(&result, 1);
        assert_eq!(le64(system_group, 0), BTRFS_DEFAULT_NODESIZE_U64);
        assert_eq!(
            le64(system_group, 16),
            BTRFS_BLOCK_GROUP_SYSTEM | BTRFS_BLOCK_GROUP_DUP
        );

        let meta_group = item_payload(&result, 2);
        assert_eq!(le64(meta_group, 0), META_BYTES_USED);
        assert_eq!(
            le64(meta_group, 16),
            BTRFS_BLOCK_GROUP_METADATA | BTRFS_BLOCK_GROUP_DUP
        );
    }

    #[test]
    fn free_space_tree_marks_only_unallocated_extents_free() {
        // ARRANGE
        let fixture = fixture();
        let context = fixture.context();

        // ACT
        let result = FreeSpaceTreeBuilder.build(&context).unwrap();

        // ASSERT
        assert_eq!(result.owner, BTRFS_FREE_SPACE_TREE_OBJECTID);
        assert_eq!(
            result.logical_offset,
            fixture.layout.meta_block(BLK_FREE_SPACE)
        );
        assert_eq!(result.items.len(), 7);
        assert_eq!(
            keys_of(&result),
            vec![
                (
                    fixture.layout.data_logical(),
                    BTRFS_FREE_SPACE_INFO_KEY,
                    BTRFS_MKFS_DATA_GROUP_SIZE
                ),
                (
                    fixture.layout.data_logical(),
                    BTRFS_FREE_SPACE_EXTENT_KEY,
                    BTRFS_MKFS_DATA_GROUP_SIZE
                ),
                (
                    fixture.layout.sys_logical(),
                    BTRFS_FREE_SPACE_INFO_KEY,
                    BTRFS_MKFS_SYSTEM_DUP_SIZE
                ),
                (
                    fixture.layout.sys_logical(),
                    BTRFS_FREE_SPACE_EXTENT_KEY,
                    BTRFS_DEFAULT_NODESIZE_U64
                ),
                (
                    fixture
                        .layout
                        .chunk_tree_logical()
                        .saturating_add(BTRFS_DEFAULT_NODESIZE_U64),
                    BTRFS_FREE_SPACE_EXTENT_KEY,
                    BTRFS_MKFS_SYSTEM_DUP_SIZE
                        .saturating_sub(BTRFS_DEFAULT_NODESIZE_U64.saturating_mul(2))
                ),
                (
                    fixture.layout.meta_logical(),
                    BTRFS_FREE_SPACE_INFO_KEY,
                    META_STRIPE
                ),
                (
                    fixture
                        .layout
                        .meta_logical()
                        .saturating_add(META_BYTES_USED),
                    BTRFS_FREE_SPACE_EXTENT_KEY,
                    META_STRIPE.saturating_sub(META_BYTES_USED)
                ),
            ]
        );
        assert_eq!(item_payload(&result, 0).len(), 8);
        assert_eq!(item_payload(&result, 2).len(), 8);
        assert_eq!(item_payload(&result, 5).len(), 8);
        assert!(item_payload(&result, 1).is_empty());
        assert!(item_payload(&result, 3).is_empty());
    }

    #[test]
    fn fs_tree_contains_default_directory_inode() {
        // ARRANGE
        let fixture = fixture();
        let context = fixture.context();

        // ACT
        let result = FsTreeBuilder::new(100).build(&context).unwrap();

        // ASSERT
        assert_eq!(result.owner, BTRFS_FS_TREE_OBJECTID);
        assert_eq!(result.logical_offset, fixture.layout.meta_block(BLK_FS));
        assert_eq!(
            keys_of(&result),
            vec![
                (256, BTRFS_INODE_ITEM_KEY, 0),
                (256, BTRFS_INODE_REF_KEY, 256),
            ]
        );

        let inode = item_payload(&result, 0);
        assert_eq!(inode.len(), 160);
        assert_eq!(le64(inode, 0), 7);
        assert_eq!(le64(inode, 112), 100);
        assert_eq!(le64(inode, 124), 100);
        assert_eq!(le64(inode, 136), 100);
        assert_eq!(le64(inode, 148), 100);

        let inode_ref = item_payload(&result, 1);
        assert_eq!(inode_ref.len(), 12);
        assert_eq!(le64(inode_ref, 0), 0);
        assert_eq!(inode_ref.get(10..12), Some(b"..".as_slice()));
    }

    #[test]
    fn data_reloc_tree_mirrors_fs_tree_shape() {
        // ARRANGE
        let fixture = fixture();
        let context = fixture.context();

        // ACT
        let result = DataRelocTreeBuilder::new(100).build(&context).unwrap();

        // ASSERT
        assert_eq!(result.owner, BTRFS_DATA_RELOC_TREE_OBJECTID);
        assert_eq!(
            result.logical_offset,
            fixture.layout.meta_block(BLK_DATA_RELOC)
        );
        assert_eq!(result.items.len(), 2);
        assert_eq!(item_payload(&result, 0).len(), 160);
        assert_eq!(item_payload(&result, 1).len(), 12);
    }

    #[test]
    fn uuid_tree_maps_fs_uuid_to_fs_tree() {
        // ARRANGE
        let fixture = fixture();
        let context = fixture.context();

        // ACT
        let result = UuidTreeBuilder.build(&context).unwrap();

        // ASSERT
        assert_eq!(result.owner, BTRFS_UUID_TREE_OBJECTID);
        assert_eq!(result.logical_offset, fixture.layout.meta_block(BLK_UUID));
        assert_eq!(
            keys_of(&result),
            vec![(FS_UUID_OBJECTID, BTRFS_UUID_KEY_SUBVOL, FS_UUID_OFFSET)]
        );
        let payload = item_payload(&result, 0);
        assert_eq!(payload.len(), 8);
        assert_eq!(le64(payload, 0), BTRFS_FS_TREE_OBJECTID);
    }

    #[test]
    fn csum_tree_starts_out_empty() {
        // ARRANGE
        let fixture = fixture();
        let context = fixture.context();

        // ACT
        let result = CsumTreeBuilder.build(&context).unwrap();

        // ASSERT
        assert_eq!(result.owner, BTRFS_CSUM_TREE_OBJECTID);
        assert_eq!(result.logical_offset, fixture.layout.meta_block(BLK_CSUM));
        assert!(result.items.is_empty());
        assert!(result.data.is_empty());
    }

    #[test]
    fn root_tree_contains_system_roots_fs_root_and_default_dir() {
        // ARRANGE
        let fixture = fixture();
        let context = fixture.context();

        // ACT
        let result = RootTreeBuilder::new(100).build(&context).unwrap();

        // ASSERT
        assert_eq!(result.owner, BTRFS_ROOT_TREE_OBJECTID);
        assert_eq!(result.logical_offset, fixture.layout.meta_block(BLK_ROOT));
        assert_eq!(result.items.len(), 12);
        assert_eq!(
            keys_of(&result),
            vec![
                (BTRFS_EXTENT_TREE_OBJECTID, BTRFS_ROOT_ITEM_KEY, 0),
                (BTRFS_DEV_TREE_OBJECTID, BTRFS_ROOT_ITEM_KEY, 0),
                (
                    BTRFS_FS_TREE_OBJECTID,
                    BTRFS_INODE_REF_KEY,
                    BTRFS_ROOT_TREE_DIR_OBJECTID
                ),
                (BTRFS_FS_TREE_OBJECTID, BTRFS_ROOT_ITEM_KEY, 0),
                (BTRFS_ROOT_TREE_DIR_OBJECTID, BTRFS_INODE_ITEM_KEY, 0),
                (
                    BTRFS_ROOT_TREE_DIR_OBJECTID,
                    BTRFS_INODE_REF_KEY,
                    BTRFS_ROOT_TREE_DIR_OBJECTID
                ),
                (
                    BTRFS_ROOT_TREE_DIR_OBJECTID,
                    BTRFS_DIR_ITEM_KEY,
                    0x8dbf_c2d2
                ),
                (BTRFS_CSUM_TREE_OBJECTID, BTRFS_ROOT_ITEM_KEY, 0),
                (BTRFS_UUID_TREE_OBJECTID, BTRFS_ROOT_ITEM_KEY, 0),
                (BTRFS_FREE_SPACE_TREE_OBJECTID, BTRFS_ROOT_ITEM_KEY, 0),
                (BTRFS_BLOCK_GROUP_TREE_OBJECTID, BTRFS_ROOT_ITEM_KEY, 0),
                (BTRFS_DATA_RELOC_TREE_OBJECTID, BTRFS_ROOT_ITEM_KEY, 0),
            ]
        );

        let extent_root = item_payload(&result, 0);
        assert_eq!(extent_root.len(), 439);
        assert_eq!(
            le64(extent_root, 176),
            fixture.layout.meta_block(BLK_EXTENT)
        );

        let fs_root = item_payload(&result, 3);
        assert_eq!(le64(fs_root, 160), 7);
        assert_eq!(le64(fs_root, 176), fixture.layout.meta_block(BLK_FS));
        assert_eq!(le64(fs_root, 64), 1_u64 << 31);
        assert_eq!(
            fs_root.get(247..263),
            Some(fixture.fs_uuid.as_bytes().as_slice())
        );
        assert_eq!(le64(fs_root, 327), 100);
        assert_eq!(le64(fs_root, 339), 100);

        let reloc_root = item_payload(&result, 11);
        assert_eq!(
            le64(reloc_root, 176),
            fixture.layout.meta_block(BLK_DATA_RELOC)
        );
        assert_eq!(reloc_root.get(247..263), Some([0_u8; 16].as_slice()));

        let dir_item = item_payload(&result, 6);
        assert_eq!(dir_item.len(), 37);
        assert_eq!(le64(dir_item, 0), BTRFS_FS_TREE_OBJECTID);
        assert_eq!(dir_item.get(8), Some(&BTRFS_ROOT_ITEM_KEY));
        assert_eq!(le64(dir_item, 9), u64::MAX);
        assert_eq!(dir_item.get(30..37), Some(b"default".as_slice()));
    }

    #[test]
    fn tree_result_empty_has_no_items_or_data() {
        // ACT
        let result = TreeResult::empty(1_024, 5);

        // ASSERT
        assert_eq!(result.logical_offset, 1_024);
        assert_eq!(result.owner, 5);
        assert!(result.items.is_empty());
        assert!(result.data.is_empty());
    }
}
