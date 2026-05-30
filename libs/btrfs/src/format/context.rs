//! Btrfs filesystem creation orchestration.
//!
//! This module provides the main `MkfsContext` that coordinates the creation
//! of all btrfs trees and the superblock.

use core::mem::size_of;
use std::fs::File;
use std::io::{Seek as _, SeekFrom, Write as _};
use std::time::{SystemTime, UNIX_EPOCH};

use uuid::Uuid;

use super::accessors::{write_disk_key, write_u16, write_u32, write_u64, write_uuid};
use super::checksum::compute_checksum;
use super::chunk::{BTRFS_BLOCK_GROUP_SYSTEM, ChunkBuilder};
use super::layout::{
    BTRFS_DEFAULT_NODESIZE, BTRFS_DEFAULT_NODESIZE_USIZE, BTRFS_DEFAULT_SECTORSIZE,
    BTRFS_DEFAULT_SECTORSIZE_U64, BTRFS_FIRST_CHUNK_TREE_OBJECTID, BTRFS_MKFS_RESERVED_SIZE_USIZE,
    BTRFS_MKFS_SYSTEM_DUP_SIZE, DiskLayout,
};
use super::structures::{
    AsBytes as _, BTRFS_CSUM_SIZE, BTRFS_LABEL_SIZE, BtrfsDevItem, BtrfsDiskKey, BtrfsHeader,
    BtrfsItem, BtrfsSuperBlock,
};
use super::trees::{BTRFS_CHUNK_ITEM_KEY, BTRFS_ROOT_TREE_DIR_OBJECTID};
use super::trees::{
    BlockGroupTreeBuilder, ChunkTreeBuilder, CsumTreeBuilder, DataRelocTreeBuilder, DevTreeBuilder,
    ExtentTreeBuilder, FreeSpaceTreeBuilder, FsTreeBuilder, RootTreeBuilder, TreeBuilder,
    TreeContext, UuidTreeBuilder,
};
use crate::error::{BtrfsError, Result};

/// Final Btrfs filesystem magic value.
const BTRFS_MAGIC: u64 = 0x4D5F_5366_5248_425F;

/// Temporary Btrfs filesystem magic written before mirrors are finalized.
const BTRFS_MAGIC_TEMPORARY: u64 = 0x4D5F_5366_5248_4221;

/// Primary superblock offset.
const BTRFS_SUPER_INFO_OFFSET: u64 = 65_536;

/// Serialized superblock size as a 64-bit byte count.
const BTRFS_SUPER_INFO_SIZE_U64: u64 = 4096;

/// Number of superblock mirror locations to consider.
const BTRFS_SUPER_MIRROR_MAX: u32 = 3;

/// Default incompatible feature set written by mkfs.
const BTRFS_FEATURE_INCOMPAT_DEFAULT: u64 = BTRFS_FEATURE_INCOMPAT_MIXED_BACKREF
    | BTRFS_FEATURE_INCOMPAT_BIG_METADATA
    | BTRFS_FEATURE_INCOMPAT_EXTENDED_IREF
    | BTRFS_FEATURE_INCOMPAT_SKINNY_METADATA
    | BTRFS_FEATURE_INCOMPAT_NO_HOLES;

/// Mixed backref incompatible feature flag.
const BTRFS_FEATURE_INCOMPAT_MIXED_BACKREF: u64 = 1 << 0;

/// Big metadata incompatible feature flag.
const BTRFS_FEATURE_INCOMPAT_BIG_METADATA: u64 = 1 << 5;

/// Extended inode reference incompatible feature flag.
const BTRFS_FEATURE_INCOMPAT_EXTENDED_IREF: u64 = 1 << 6;

/// Skinny metadata incompatible feature flag.
const BTRFS_FEATURE_INCOMPAT_SKINNY_METADATA: u64 = 1 << 8;

/// No-holes incompatible feature flag.
const BTRFS_FEATURE_INCOMPAT_NO_HOLES: u64 = 1 << 9;

/// Default read-only-compatible feature set written by mkfs.
const BTRFS_FEATURE_COMPAT_RO_DEFAULT: u64 = BTRFS_FEATURE_COMPAT_RO_FREE_SPACE_TREE
    | BTRFS_FEATURE_COMPAT_RO_FREE_SPACE_TREE_VALID
    | BTRFS_FEATURE_COMPAT_RO_BLOCK_GROUP_TREE;

/// Free-space tree read-only-compatible feature flag.
const BTRFS_FEATURE_COMPAT_RO_FREE_SPACE_TREE: u64 = 1 << 0;

/// Valid free-space tree read-only-compatible feature flag.
const BTRFS_FEATURE_COMPAT_RO_FREE_SPACE_TREE_VALID: u64 = 1 << 1;

/// Block-group tree read-only-compatible feature flag.
const BTRFS_FEATURE_COMPAT_RO_BLOCK_GROUP_TREE: u64 = 1 << 3;

/// CRC32C checksum type value.
const BTRFS_CSUM_TYPE_CRC32: u16 = 0;

/// Tree header written flag.
const BTRFS_HEADER_FLAG_WRITTEN: u64 = 1 << 0;

/// Shift for the backref revision in tree header flags.
const BTRFS_BACKREF_REV_SHIFT: u64 = 56;

/// Mixed backref revision value stored in tree header flags.
const BTRFS_MIXED_BACKREF_REV: u64 = 1;

/// Superblock written flag.
const BTRFS_SUPER_FLAG_WRITTEN: u64 = 1 << 0;

/// Superblock mirror offset for the requested mirror index.
const fn btrfs_sb_offset(mirror: u32) -> u64 {
    match mirror {
        0 => BTRFS_SUPER_INFO_OFFSET,
        1 => 67_108_864,
        _ => 274_877_906_944,
    }
}

/// Context for creating a btrfs filesystem.
///
/// This struct orchestrates the creation of all btrfs structures
/// but delegates the actual tree building to specialized builders.
pub struct MkfsContext {
    device: File,
    device_size: u64,
    label: String,
    fsid: Uuid,
    chunk_uuid: Uuid,
    dev_uuid: Uuid,
    fs_uuid: Uuid,
    generation: u64,
    layout: DiskLayout,
}

impl MkfsContext {
    /// Create a new mkfs context.
    ///
    /// # Arguments
    /// * `device` - The device file to write to
    /// * `device_size` - Size of the device in bytes
    /// * `label` - Filesystem label
    #[must_use]
    pub fn new(device: File, device_size: u64, label: String) -> Self {
        let device_size = device_size
            .checked_div(BTRFS_DEFAULT_SECTORSIZE_U64)
            .unwrap_or(0)
            .saturating_mul(BTRFS_DEFAULT_SECTORSIZE_U64);
        let layout = DiskLayout::new(device_size);
        Self {
            device,
            device_size,
            label,
            fsid: Uuid::new_v4(),
            chunk_uuid: Uuid::new_v4(),
            dev_uuid: Uuid::new_v4(),
            fs_uuid: Uuid::new_v4(),
            generation: 1,
            layout,
        }
    }

    /// Write data to a specific physical offset.
    fn write_at(&mut self, buf: &[u8], phys: u64) -> Result<()> {
        self.device.seek(SeekFrom::Start(phys))?;
        self.device.write_all(buf)?;
        Ok(())
    }

    /// Build a leaf node with header and checksum.
    fn build_node(
        &self,
        offset: u64,
        owner: u64,
        items: &[BtrfsItem],
        data: &[u8],
    ) -> Result<Vec<u8>> {
        let mut node = vec![0_u8; BTRFS_DEFAULT_NODESIZE_USIZE];

        let mut header = BtrfsHeader::new();
        write_u64(&mut header.bytenr, offset);
        write_u64(
            &mut header.flags,
            BTRFS_HEADER_FLAG_WRITTEN | (BTRFS_MIXED_BACKREF_REV << BTRFS_BACKREF_REV_SHIFT),
        );
        write_uuid(&mut header.chunk_tree_uuid, &self.chunk_uuid);
        write_u64(&mut header.generation, self.generation);
        write_u64(&mut header.owner, owner);
        let item_count = u32::try_from(items.len())
            .map_err(|_error| BtrfsError::Mkfs("too many leaf items".to_owned()))?;
        write_u32(&mut header.nritems, item_count);
        header.level = 0;
        write_uuid(&mut header.fsid, &self.fsid);

        let hdr_bytes = header.as_bytes();
        let hdr_tail = hdr_bytes
            .get(BTRFS_CSUM_SIZE..)
            .ok_or_else(|| BtrfsError::Mkfs("invalid header checksum range".to_owned()))?;
        copy_into(&mut node, BTRFS_CSUM_SIZE, hdr_tail)?;

        let items_start = size_of::<BtrfsHeader>();
        let item_size = size_of::<BtrfsItem>();
        for (index, item) in items.iter().enumerate() {
            let item_offset = index
                .checked_mul(item_size)
                .ok_or_else(|| BtrfsError::Mkfs("leaf item offset overflow".to_owned()))?;
            let start = items_start
                .checked_add(item_offset)
                .ok_or_else(|| BtrfsError::Mkfs("leaf item offset overflow".to_owned()))?;
            copy_into(&mut node, start, item.as_bytes())?;
        }

        if !data.is_empty() {
            let data_start = BTRFS_DEFAULT_NODESIZE_USIZE
                .checked_sub(data.len())
                .ok_or_else(|| BtrfsError::Mkfs("leaf data exceeds node size".to_owned()))?;
            copy_into(&mut node, data_start, data)?;
        }

        let checksum_input = node
            .get(BTRFS_CSUM_SIZE..)
            .ok_or_else(|| BtrfsError::Mkfs("invalid checksum range".to_owned()))?;
        let csum = compute_checksum(checksum_input);
        copy_into(&mut node, 0, &csum)?;

        Ok(node)
    }

    /// Write a leaf to the metadata DUP chunk (both stripes).
    fn write_meta_leaf(
        &mut self,
        logical: u64,
        owner: u64,
        items: &[BtrfsItem],
        data: &[u8],
    ) -> Result<()> {
        let node = self.build_node(logical, owner, items, data)?;
        let phys_0 = self.layout.meta_logical_to_phys(logical);
        let phys_1 = phys_0.saturating_add(self.layout.meta_stripe_size());
        self.write_at(&node, phys_0)?;
        self.write_at(&node, phys_1)?;
        Ok(())
    }

    /// Create the chunk tree and return the system chunk data for the superblock.
    fn make_chunk_tree(&mut self) -> Result<Vec<u8>> {
        let builder = ChunkTreeBuilder::new();
        let result = builder.build(&self.create_context())?;

        // Write to system chunk
        let node = self.build_node(
            result.logical_offset,
            result.owner,
            &result.items,
            &result.data,
        )?;
        let phys_0 = self.layout.sys_logical_to_phys(result.logical_offset);
        let phys_1 = phys_0.saturating_add(BTRFS_MKFS_SYSTEM_DUP_SIZE);
        self.write_at(&node, phys_0)?;
        self.write_at(&node, phys_1)?;

        // Return system chunk data for superblock
        // The system chunk is the second item in the chunk tree
        // We need to extract it from the built data
        // This is a bit of a hack - we rebuild just the system chunk
        let sys_chunk = ChunkBuilder::dup(
            BTRFS_MKFS_SYSTEM_DUP_SIZE,
            BTRFS_BLOCK_GROUP_SYSTEM,
            self.layout.sys_phys_0(),
            self.layout.sys_phys_1(),
        )
        .build(&self.dev_uuid)?;

        Ok(sys_chunk)
    }

    /// Create a tree context.
    fn create_context(&self) -> TreeContext<'_> {
        TreeContext::new(
            &self.layout,
            &self.fsid,
            &self.chunk_uuid,
            &self.dev_uuid,
            &self.fs_uuid,
            self.generation,
            self.device_size,
        )
    }

    /// Build and write a tree using the provided builder.
    fn build_and_write_tree<T: TreeBuilder>(&mut self, builder: &T) -> Result<()> {
        let ctx = self.create_context();
        let result = builder.build(&ctx)?;
        self.write_meta_leaf(
            result.logical_offset,
            result.owner,
            &result.items,
            &result.data,
        )
    }

    /// Create all metadata trees.
    fn make_trees(&mut self, now: u64) -> Result<()> {
        self.build_and_write_tree(&DevTreeBuilder::new())?;
        self.build_and_write_tree(&ExtentTreeBuilder::new())?;
        self.build_and_write_tree(&BlockGroupTreeBuilder::new())?;
        self.build_and_write_tree(&FreeSpaceTreeBuilder::new())?;
        self.build_and_write_tree(&FsTreeBuilder::new(now))?;
        self.build_and_write_tree(&DataRelocTreeBuilder::new(now))?;
        self.build_and_write_tree(&UuidTreeBuilder::new())?;
        self.build_and_write_tree(&CsumTreeBuilder::new())?;
        self.build_and_write_tree(&RootTreeBuilder::new(now))?;

        Ok(())
    }

    /// Write the superblock to the device.
    fn write_superblock(&mut self, sb: &mut BtrfsSuperBlock, bytenr: u64) -> Result<()> {
        write_u64(&mut sb.bytenr, bytenr);

        let sb_bytes = sb.as_bytes();
        let checksum_input = sb_bytes
            .get(BTRFS_CSUM_SIZE..)
            .ok_or_else(invalid_superblock_checksum_range)?;
        let csum = compute_checksum(checksum_input);
        copy_into(&mut sb.csum, 0, &csum)?;

        self.device.seek(SeekFrom::Start(bytenr))?;
        self.device.write_all(sb.as_bytes())?;
        Ok(())
    }

    /// Write superblock mirrors.
    fn write_superblock_mirrors(&mut self, sb: &mut BtrfsSuperBlock) -> Result<()> {
        let offsets: Vec<u64> = (0..BTRFS_SUPER_MIRROR_MAX)
            .map(btrfs_sb_offset)
            .take_while(|&bytenr| {
                bytenr.saturating_add(BTRFS_SUPER_INFO_SIZE_U64) <= self.device_size
            })
            .collect();

        for bytenr in offsets {
            self.write_superblock(sb, bytenr)?;
        }
        Ok(())
    }

    /// Create and write the superblock.
    fn make_superblock(&mut self, sys_chunk_data: &[u8]) -> Result<()> {
        let mut sb = BtrfsSuperBlock::new();

        write_u64(&mut sb.flags, BTRFS_SUPER_FLAG_WRITTEN);
        write_u64(&mut sb.magic, BTRFS_MAGIC_TEMPORARY);
        write_u64(&mut sb.generation, self.generation);
        write_u64(
            &mut sb.root,
            self.layout.meta_block(super::layout::BLK_ROOT),
        );
        write_u64(&mut sb.chunk_root, self.layout.chunk_tree_logical());
        write_u64(&mut sb.total_bytes, self.device_size);
        write_u64(&mut sb.bytes_used, DiskLayout::total_bytes_used());
        write_u64(&mut sb.root_dir_objectid, BTRFS_ROOT_TREE_DIR_OBJECTID);
        write_u64(&mut sb.num_devices, 1);
        write_u32(&mut sb.sectorsize, BTRFS_DEFAULT_SECTORSIZE);
        write_u32(&mut sb.nodesize, BTRFS_DEFAULT_NODESIZE);
        write_u32(&mut sb.leafsize, BTRFS_DEFAULT_NODESIZE);
        write_u32(&mut sb.stripesize, BTRFS_DEFAULT_SECTORSIZE);
        write_u64(&mut sb.chunk_root_generation, self.generation);
        write_u64(&mut sb.compat_ro_flags, BTRFS_FEATURE_COMPAT_RO_DEFAULT);
        write_u64(&mut sb.incompat_flags, BTRFS_FEATURE_INCOMPAT_DEFAULT);
        write_u16(&mut sb.csum_type, BTRFS_CSUM_TYPE_CRC32);

        write_uuid(&mut sb.fsid, &self.fsid);

        // Dev item
        let mut dev = BtrfsDevItem::new();
        write_u64(&mut dev.devid, 1);
        write_u64(&mut dev.total_bytes, self.device_size);
        write_u64(&mut dev.bytes_used, self.layout.dev_bytes_used());
        write_u32(&mut dev.io_align, BTRFS_DEFAULT_SECTORSIZE);
        write_u32(&mut dev.io_width, BTRFS_DEFAULT_SECTORSIZE);
        write_u32(&mut dev.sector_size, BTRFS_DEFAULT_SECTORSIZE);
        write_uuid(&mut dev.uuid, &self.dev_uuid);
        write_uuid(&mut dev.fsid, &self.fsid);
        sb.dev_item = dev;

        // Label
        let label_bytes = self.label.as_bytes();
        let len = label_bytes.len().min(BTRFS_LABEL_SIZE.saturating_sub(1));
        let label = label_bytes
            .get(..len)
            .ok_or_else(|| BtrfsError::Mkfs("invalid label range".to_owned()))?;
        copy_into(&mut sb.label, 0, label)?;

        // sys_chunk_array: key + DUP system chunk data
        let mut key = BtrfsDiskKey {
            objectid: [0; 8],
            type_: 0,
            offset: [0; 8],
        };
        write_disk_key(
            &mut key,
            BTRFS_FIRST_CHUNK_TREE_OBJECTID,
            BTRFS_CHUNK_ITEM_KEY,
            self.layout.sys_logical(),
        );
        let sys_chunk_array_size = size_of::<BtrfsDiskKey>()
            .checked_add(sys_chunk_data.len())
            .ok_or_else(|| BtrfsError::Mkfs("system chunk array size overflow".to_owned()))?;
        let mut array_buf = key.to_vec();
        array_buf.extend_from_slice(sys_chunk_data);
        copy_into(&mut sb.sys_chunk_array, 0, &array_buf)?;
        let sys_chunk_array_size = u32::try_from(sys_chunk_array_size)
            .map_err(|_error| BtrfsError::Mkfs("system chunk array is too large".to_owned()))?;
        write_u32(&mut sb.sys_chunk_array_size, sys_chunk_array_size);

        // Phase 1: temporary magic
        self.write_superblock(&mut sb, BTRFS_SUPER_INFO_OFFSET)?;

        // Phase 2: real magic on all mirrors
        write_u64(&mut sb.magic, BTRFS_MAGIC);
        self.write_superblock_mirrors(&mut sb)?;

        Ok(())
    }

    /// Create the btrfs filesystem.
    ///
    /// # Errors
    /// Returns an error if the device is too small or any filesystem write fails.
    pub fn make_btrfs(&mut self) -> Result<()> {
        let min_size = self.layout.min_device_size();
        if self.device_size < min_size {
            return Err(BtrfsError::DeviceTooSmall {
                min_size,
                actual_size: self.device_size,
            });
        }

        let zeros = vec![0_u8; BTRFS_MKFS_RESERVED_SIZE_USIZE];
        self.device.seek(SeekFrom::Start(0))?;
        self.device.write_all(&zeros)?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let sys_chunk = self.make_chunk_tree()?;

        self.make_trees(now)?;
        self.make_superblock(&sys_chunk)?;
        self.device.sync_all()?;

        Ok(())
    }
}

fn copy_into(dest: &mut [u8], offset: usize, src: &[u8]) -> Result<()> {
    let end = offset
        .checked_add(src.len())
        .ok_or_else(|| BtrfsError::Mkfs("copy offset overflow".to_owned()))?;
    let dest = dest
        .get_mut(offset..end)
        .ok_or_else(|| BtrfsError::Mkfs("copy exceeds destination buffer".to_owned()))?;
    dest.copy_from_slice(src);
    Ok(())
}

fn invalid_superblock_checksum_range() -> BtrfsError {
    BtrfsError::Mkfs("invalid superblock checksum range".to_owned())
}
