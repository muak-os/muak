//! Btrfs filesystem creation orchestration.
//!
//! This module provides the main `MkfsContext` that coordinates the creation
//! of all btrfs trees and the superblock.

use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use uuid::Uuid;

use super::accessors::*;
use super::checksum::compute_checksum;
use super::constants::*;
use super::layout::DiskLayout;
use super::structures::*;
use super::trees::*;
use crate::error::{BtrfsError, Result};

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
    pub fn new(device: File, device_size: u64, label: String) -> Self {
        let device_size =
            (device_size / BTRFS_DEFAULT_SECTORSIZE as u64) * BTRFS_DEFAULT_SECTORSIZE as u64;
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
    fn build_node(&self, offset: u64, owner: u64, items: &[BtrfsItem], data: &[u8]) -> Vec<u8> {
        let mut node = vec![0u8; BTRFS_DEFAULT_NODESIZE as usize];

        let mut header = BtrfsHeader::new();
        write_u64(&mut header.bytenr, offset);
        write_u64(
            &mut header.flags,
            BTRFS_HEADER_FLAG_WRITTEN | (BTRFS_MIXED_BACKREF_REV << BTRFS_BACKREF_REV_SHIFT),
        );
        write_uuid(&mut header.chunk_tree_uuid, &self.chunk_uuid);
        write_u64(&mut header.generation, self.generation);
        write_u64(&mut header.owner, owner);
        write_u32(&mut header.nritems, items.len() as u32);
        header.level = 0;
        write_uuid(&mut header.fsid, &self.fsid);

        let hdr_bytes = header.as_bytes();
        node[BTRFS_CSUM_SIZE..hdr_bytes.len()].copy_from_slice(&hdr_bytes[BTRFS_CSUM_SIZE..]);

        let items_start = std::mem::size_of::<BtrfsHeader>();
        for (i, item) in items.iter().enumerate() {
            let start = items_start + i * std::mem::size_of::<BtrfsItem>();
            node[start..start + std::mem::size_of::<BtrfsItem>()].copy_from_slice(item.as_bytes());
        }

        if !data.is_empty() {
            let data_start = BTRFS_DEFAULT_NODESIZE as usize - data.len();
            node[data_start..data_start + data.len()].copy_from_slice(data);
        }

        let csum = compute_checksum(&node[BTRFS_CSUM_SIZE..]);
        node[..4].copy_from_slice(&csum);

        node
    }

    /// Write a leaf to the metadata DUP chunk (both stripes).
    fn write_meta_leaf(
        &mut self,
        logical: u64,
        owner: u64,
        items: &[BtrfsItem],
        data: &[u8],
    ) -> Result<()> {
        let node = self.build_node(logical, owner, items, data);
        let phys_0 = self.layout.meta_logical_to_phys(logical);
        let phys_1 = phys_0 + self.layout.meta_stripe_size();
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
        );
        let phys_0 = self.layout.sys_logical_to_phys(result.logical_offset);
        let phys_1 = phys_0 + BTRFS_MKFS_SYSTEM_DUP_SIZE;
        self.write_at(&node, phys_0)?;
        self.write_at(&node, phys_1)?;

        // Return system chunk data for superblock
        // The system chunk is the second item in the chunk tree
        // We need to extract it from the built data
        // This is a bit of a hack - we rebuild just the system chunk
        let sys_chunk = super::chunk::ChunkBuilder::dup(
            BTRFS_MKFS_SYSTEM_DUP_SIZE,
            BTRFS_BLOCK_GROUP_SYSTEM,
            self.layout.sys_phys_0(),
            self.layout.sys_phys_1(),
        )
        .build(&self.dev_uuid);

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
    fn build_and_write_tree<T: TreeBuilder>(&mut self, builder: T, _tree_name: &str) -> Result<()> {
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
        self.build_and_write_tree(DevTreeBuilder::new(), "dev tree")?;
        self.build_and_write_tree(ExtentTreeBuilder::new(), "extent tree")?;
        self.build_and_write_tree(BlockGroupTreeBuilder::new(), "block group tree")?;
        self.build_and_write_tree(FreeSpaceTreeBuilder::new(), "free space tree")?;
        self.build_and_write_tree(FsTreeBuilder::new(now), "FS tree")?;
        self.build_and_write_tree(DataRelocTreeBuilder::new(now), "data reloc tree")?;
        self.build_and_write_tree(UuidTreeBuilder::new(), "UUID tree")?;
        self.build_and_write_tree(CsumTreeBuilder::new(), "CSUM tree")?;
        self.build_and_write_tree(RootTreeBuilder::new(now), "root tree")?;

        Ok(())
    }

    /// Write the superblock to the device.
    fn write_superblock(&mut self, sb: &mut BtrfsSuperBlock, bytenr: u64) -> Result<()> {
        write_u64(&mut sb.bytenr, bytenr);

        let sb_bytes = sb.as_bytes();
        let csum = compute_checksum(&sb_bytes[BTRFS_CSUM_SIZE..]);
        sb.csum[..4].copy_from_slice(&csum);

        self.device.seek(SeekFrom::Start(bytenr))?;
        self.device.write_all(sb.as_bytes())?;
        Ok(())
    }

    /// Write superblock mirrors.
    fn write_superblock_mirrors(&mut self, sb: &mut BtrfsSuperBlock) -> Result<()> {
        let offsets: Vec<u64> = (0..BTRFS_SUPER_MIRROR_MAX)
            .map(btrfs_sb_offset)
            .take_while(|&bytenr| bytenr + BTRFS_SUPER_INFO_SIZE as u64 <= self.device_size)
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
        write_u64(&mut sb.bytes_used, self.layout.total_bytes_used());
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
        let len = label_bytes.len().min(BTRFS_LABEL_SIZE - 1);
        sb.label[..len].copy_from_slice(&label_bytes[..len]);

        // sys_chunk_array: key + DUP system chunk data
        let mut key = BtrfsDiskKey {
            objectid: [0; 8],
            type_: 0,
            offset: [0; 8],
        };
        super::accessors::write_disk_key(
            &mut key,
            BTRFS_FIRST_CHUNK_TREE_OBJECTID,
            BTRFS_CHUNK_ITEM_KEY,
            self.layout.sys_logical(),
        );
        let sys_chunk_array_size = std::mem::size_of::<BtrfsDiskKey>() + sys_chunk_data.len();
        let mut array_buf = key.to_vec();
        array_buf.extend_from_slice(sys_chunk_data);
        sb.sys_chunk_array[..sys_chunk_array_size].copy_from_slice(&array_buf);
        super::accessors::write_u32(&mut sb.sys_chunk_array_size, sys_chunk_array_size as u32);

        // Phase 1: temporary magic
        self.write_superblock(&mut sb, BTRFS_SUPER_INFO_OFFSET)?;

        // Phase 2: real magic on all mirrors
        write_u64(&mut sb.magic, BTRFS_MAGIC);
        self.write_superblock_mirrors(&mut sb)?;

        Ok(())
    }

    /// Create the btrfs filesystem.
    pub fn make_btrfs(&mut self) -> Result<()> {
        let min_size = self.layout.min_device_size();
        if self.device_size < min_size {
            return Err(BtrfsError::DeviceTooSmall {
                min_size,
                actual_size: self.device_size,
            });
        }

        let zeros = vec![0u8; BTRFS_MKFS_RESERVED_SIZE as usize];
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
