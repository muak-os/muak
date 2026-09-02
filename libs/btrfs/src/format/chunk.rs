//! Chunk building utilities for btrfs filesystem creation.
//!
//! This module provides a unified interface for building chunk items,
//! eliminating duplication between SINGLE and DUP chunk creation.

use uuid::Uuid;

use super::accessors::{write_u16, write_u32, write_u64, write_uuid};
use super::layout::{BTRFS_DEFAULT_SECTORSIZE, BTRFS_EXTENT_TREE_OBJECTID, BTRFS_STRIPE_LEN};
use super::structures::{AsBytes as _, BtrfsChunk, BtrfsStripe};
use crate::error::{BtrfsError, Result};

/// Data block-group profile flag.
pub(super) const BTRFS_BLOCK_GROUP_DATA: u64 = 1 << 0;

/// System block-group profile flag.
pub(super) const BTRFS_BLOCK_GROUP_SYSTEM: u64 = 1 << 1;

/// Metadata block-group profile flag.
pub(super) const BTRFS_BLOCK_GROUP_METADATA: u64 = 1 << 2;

/// DUP block-group profile flag.
pub(super) const BTRFS_BLOCK_GROUP_DUP: u64 = 1 << 5;

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
    #[must_use]
    pub fn new(length: u64, chunk_type: u64) -> Self {
        Self {
            length,
            chunk_type,
            stripes: Vec::new(),
        }
    }

    /// Create a SINGLE chunk (one stripe).
    #[must_use]
    pub fn single(length: u64, chunk_type: u64, physical_offset: u64) -> Self {
        let mut builder = Self::new(length, chunk_type);
        builder.add_stripe(1, physical_offset);
        builder
    }

    /// Create a DUP chunk (two stripes on the same device).
    #[must_use]
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
    pub fn build(&self, dev_uuid: &Uuid) -> Result<Vec<u8>> {
        let num_stripes = self.stripes.len();
        let stripe_bytes = num_stripes
            .checked_mul(size_of::<BtrfsStripe>())
            .ok_or_else(|| BtrfsError::Mkfs("chunk stripe buffer size overflow".to_owned()))?;
        let buf_size = size_of::<BtrfsChunk>()
            .checked_add(stripe_bytes)
            .ok_or_else(|| BtrfsError::Mkfs("chunk buffer size overflow".to_owned()))?;
        let mut buf = Vec::with_capacity(buf_size);

        let mut chunk = BtrfsChunk {
            length: [0; 8],
            owner: [0; 8],
            stripe_len: [0; 8],
            type_: [0; 8],
            io_align: [0; 4],
            io_width: [0; 4],
            sector_size: [0; 4],
            num_stripes: [0; 2],
            sub_stripes: [0; 2],
        };
        write_u64(&mut chunk.length, self.length);
        write_u64(&mut chunk.owner, BTRFS_EXTENT_TREE_OBJECTID);
        write_u64(&mut chunk.stripe_len, BTRFS_STRIPE_LEN);
        write_u64(&mut chunk.type_, self.chunk_type);
        write_u32(&mut chunk.io_align, BTRFS_DEFAULT_SECTORSIZE);
        write_u32(&mut chunk.io_width, BTRFS_DEFAULT_SECTORSIZE);
        write_u32(&mut chunk.sector_size, BTRFS_DEFAULT_SECTORSIZE);
        let num_stripes = u16::try_from(num_stripes)
            .map_err(|_error| BtrfsError::Mkfs("too many chunk stripes".to_owned()))?;
        write_u16(&mut chunk.num_stripes, num_stripes);
        write_u16(&mut chunk.sub_stripes, 0);
        buf.extend_from_slice(chunk.as_bytes());

        for stripe_config in &self.stripes {
            let mut stripe = BtrfsStripe {
                devid: [0; 8],
                offset: [0; 8],
                dev_uuid: [0; 16],
            };
            write_u64(&mut stripe.devid, stripe_config.devid);
            write_u64(&mut stripe.offset, stripe_config.offset);
            write_uuid(&mut stripe.dev_uuid, dev_uuid);
            buf.extend_from_slice(stripe.as_bytes());
        }

        Ok(buf)
    }
}

impl Default for ChunkBuilder {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    fn dev_uuid() -> Uuid {
        Uuid::from_u128(0x1122_3344_5566_7788_99AA_BBCC_DDEE_FF00)
    }

    fn le64(data: &[u8], at: usize) -> u64 {
        u64::from_le_bytes(
            data.get(at..at.saturating_add(8))
                .unwrap()
                .try_into()
                .unwrap(),
        )
    }

    fn le32(data: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(
            data.get(at..at.saturating_add(4))
                .unwrap()
                .try_into()
                .unwrap(),
        )
    }

    fn le16(data: &[u8], at: usize) -> u16 {
        u16::from_le_bytes(
            data.get(at..at.saturating_add(2))
                .unwrap()
                .try_into()
                .unwrap(),
        )
    }

    #[test]
    fn single_chunk_encodes_header_and_one_stripe() {
        // ARRANGE
        let builder = ChunkBuilder::single(8_388_608, BTRFS_BLOCK_GROUP_DATA, 0x2000_0000);

        // ACT
        let bytes = builder.build(&dev_uuid()).unwrap();

        // ASSERT
        assert_eq!(bytes.len(), 80);
        assert_eq!(le64(&bytes, 0), 8_388_608);
        assert_eq!(le64(&bytes, 8), BTRFS_EXTENT_TREE_OBJECTID);
        assert_eq!(le64(&bytes, 16), BTRFS_STRIPE_LEN);
        assert_eq!(le64(&bytes, 24), BTRFS_BLOCK_GROUP_DATA);
        assert_eq!(le32(&bytes, 32), BTRFS_DEFAULT_SECTORSIZE);
        assert_eq!(le32(&bytes, 36), BTRFS_DEFAULT_SECTORSIZE);
        assert_eq!(le32(&bytes, 40), BTRFS_DEFAULT_SECTORSIZE);
        assert_eq!(le16(&bytes, 44), 1);
        assert_eq!(le16(&bytes, 46), 0);
        assert_eq!(le64(&bytes, 48), 1);
        assert_eq!(le64(&bytes, 56), 0x2000_0000);
        assert_eq!(bytes.get(64..80), Some(dev_uuid().as_bytes().as_slice()));
    }

    #[test]
    fn dup_chunk_sets_flag_and_two_stripes() {
        // ARRANGE
        let builder = ChunkBuilder::dup(
            8_388_608,
            BTRFS_BLOCK_GROUP_SYSTEM,
            0x1000_0000,
            0x1400_0000,
        );

        // ACT
        let bytes = builder.build(&dev_uuid()).unwrap();

        // ASSERT
        assert_eq!(bytes.len(), 112);
        assert_eq!(
            le64(&bytes, 24),
            BTRFS_BLOCK_GROUP_SYSTEM | BTRFS_BLOCK_GROUP_DUP
        );
        assert_eq!(le16(&bytes, 44), 2);
        assert_eq!(le64(&bytes, 48), 1);
        assert_eq!(le64(&bytes, 56), 0x1000_0000);
        assert_eq!(le64(&bytes, 80), 1);
        assert_eq!(le64(&bytes, 88), 0x1400_0000);
    }

    #[test]
    fn add_stripe_preserves_insertion_order() {
        // ARRANGE
        let mut builder = ChunkBuilder::new(1_024, BTRFS_BLOCK_GROUP_METADATA);
        builder.add_stripe(3, 100).add_stripe(4, 200);

        // ACT
        let bytes = builder.build(&dev_uuid()).unwrap();

        // ASSERT
        assert_eq!(le64(&bytes, 48), 3);
        assert_eq!(le64(&bytes, 80), 4);
    }

    #[test]
    fn default_builder_serializes_zeroed_chunk() {
        // ARRANGE
        let builder = ChunkBuilder::default();

        // ACT
        let bytes = builder.build(&dev_uuid()).unwrap();

        // ASSERT
        assert_eq!(bytes.len(), 48);
        assert_eq!(le64(&bytes, 0), 0);
        assert_eq!(le16(&bytes, 44), 0);
    }

    #[test]
    fn build_rejects_more_stripes_than_fit_u16() {
        // ARRANGE
        let mut builder = ChunkBuilder::new(1, 0);
        for devid in 0..=u64::from(u16::MAX) {
            builder.add_stripe(devid, 0);
        }

        // ACT
        let result = builder.build(&dev_uuid());

        // ASSERT
        assert!(matches!(result, Err(BtrfsError::Mkfs(_))));
    }
}
