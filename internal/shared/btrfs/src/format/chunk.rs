//! Chunk building utilities for btrfs filesystem creation.
//!
//! This module provides a unified interface for building chunk items,
//! eliminating duplication between SINGLE and DUP chunk creation.

use super::accessors::*;
use super::constants::*;
use super::structures::*;
use uuid::Uuid;

/// Configuration for a single stripe.
#[derive(Debug, Clone)]
pub struct StripeConfig {
    pub devid: u64,
    pub offset: u64,
}

/// Builder for btrfs chunk items.
#[derive(Debug)]
pub struct ChunkBuilder {
    length: u64,
    chunk_type: u64,
    stripes: Vec<StripeConfig>,
}

impl ChunkBuilder {
    /// Create a new chunk builder with the given length.
    pub fn new(length: u64, chunk_type: u64) -> Self {
        Self {
            length,
            chunk_type,
            stripes: Vec::new(),
        }
    }

    /// Create a SINGLE chunk (one stripe).
    pub fn single(length: u64, chunk_type: u64, physical_offset: u64) -> Self {
        let mut builder = Self::new(length, chunk_type);
        builder.add_stripe(1, physical_offset);
        builder
    }

    /// Create a DUP chunk (two stripes on the same device).
    pub fn dup(length: u64, chunk_type: u64, phys_offset_0: u64, phys_offset_1: u64) -> Self {
        let mut builder = Self::new(length, chunk_type | BTRFS_BLOCK_GROUP_DUP);
        builder.add_stripe(1, phys_offset_0);
        builder.add_stripe(1, phys_offset_1);
        builder
    }

    /// Add a stripe to this chunk.
    pub fn add_stripe(&mut self, devid: u64, offset: u64) -> &mut Self {
        self.stripes.push(StripeConfig { devid, offset });
        self
    }

    /// Build the chunk item data.
    pub fn build(&self, dev_uuid: &Uuid) -> Vec<u8> {
        let num_stripes = self.stripes.len();
        let buf_size = size_of::<BtrfsChunk>() + num_stripes * size_of::<BtrfsStripe>();
        let mut buf = vec![0u8; buf_size];

        // Build chunk header
        let chunk = unsafe { &mut *(buf.as_mut_ptr() as *mut BtrfsChunk) };
        write_u64(&mut chunk.length, self.length);
        write_u64(&mut chunk.owner, BTRFS_EXTENT_TREE_OBJECTID);
        write_u64(&mut chunk.stripe_len, BTRFS_STRIPE_LEN);
        write_u64(&mut chunk.type_, self.chunk_type);
        write_u32(&mut chunk.io_align, BTRFS_DEFAULT_SECTORSIZE);
        write_u32(&mut chunk.io_width, BTRFS_DEFAULT_SECTORSIZE);
        write_u32(&mut chunk.sector_size, BTRFS_DEFAULT_SECTORSIZE);
        write_u16(&mut chunk.num_stripes, num_stripes as u16);
        write_u16(&mut chunk.sub_stripes, 0);

        // Build stripes
        let stripe_offset = size_of::<BtrfsChunk>();
        for (i, stripe) in self.stripes.iter().enumerate() {
            let stripe_ptr = unsafe {
                &mut *(buf
                    .as_mut_ptr()
                    .add(stripe_offset + i * size_of::<BtrfsStripe>())
                    as *mut BtrfsStripe)
            };
            write_u64(&mut stripe_ptr.devid, stripe.devid);
            write_u64(&mut stripe_ptr.offset, stripe.offset);
            write_uuid(&mut stripe_ptr.dev_uuid, dev_uuid);
        }

        buf
    }
}

impl Default for ChunkBuilder {
    fn default() -> Self {
        Self::new(0, 0)
    }
}
