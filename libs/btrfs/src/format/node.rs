//! Node and leaf building utilities for Btrfs filesystem creation.

use super::accessors::{write_disk_key, write_u32};
use super::layout::BTRFS_DEFAULT_NODESIZE_USIZE;
use super::structures::{BtrfsDiskKey, BtrfsItem};
use crate::error::{BtrfsError, Result};

/// Offset where leaf item data begins after the header and item array.
const BTRFS_LEAF_DATA_OFFSET: usize = 101;

/// Builds Btrfs leaf nodes with properly sorted items.
#[derive(Debug)]
pub struct LeafBuilder {
    items: Vec<(u64, u8, u64, Vec<u8>)>,
}

impl LeafBuilder {
    /// Create a new empty leaf builder.
    #[must_use]
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Add an item to the leaf.
    pub fn add_item(&mut self, objectid: u64, type_: u8, offset: u64, data: Vec<u8>) {
        self.items.push((objectid, type_, offset, data));
    }

    /// Build the leaf items and data.
    ///
    /// Returns a tuple of (items, data) where items are the `BtrfsItem` structs
    /// and data is the concatenated item data.
    ///
    /// # Errors
    /// Returns an error if item data does not fit in a leaf or offsets exceed `u32`.
    pub fn build(mut self) -> Result<(Vec<BtrfsItem>, Vec<u8>)> {
        self.items.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then(left.1.cmp(&right.1))
                .then(left.2.cmp(&right.2))
        });

        let mut result_items = Vec::with_capacity(self.items.len());
        let total_data_len = self
            .items
            .iter()
            .try_fold(0_usize, |total, item| total.checked_add(item.3.len()))
            .ok_or_else(|| BtrfsError::Mkfs("leaf data size overflow".to_owned()))?;
        let mut data_chunks = Vec::with_capacity(self.items.len());
        let mut current_offset = BTRFS_DEFAULT_NODESIZE_USIZE
            .checked_sub(BTRFS_LEAF_DATA_OFFSET)
            .ok_or_else(|| BtrfsError::Mkfs("invalid leaf data offset".to_owned()))?;

        for (objectid, type_, offset, data) in self.items {
            current_offset = current_offset
                .checked_sub(data.len())
                .ok_or_else(|| BtrfsError::Mkfs("leaf data exceeds node size".to_owned()))?;

            let mut item = BtrfsItem {
                key: BtrfsDiskKey {
                    objectid: [0; 8],
                    type_: 0,
                    offset: [0; 8],
                },
                offset: [0; 4],
                size: [0; 4],
            };
            write_disk_key(&mut item.key, objectid, type_, offset);
            let item_offset = u32::try_from(current_offset)
                .map_err(|_error| BtrfsError::Mkfs("leaf item offset exceeds u32".to_owned()))?;
            let item_size = u32::try_from(data.len())
                .map_err(|_error| BtrfsError::Mkfs("leaf item size exceeds u32".to_owned()))?;
            write_u32(&mut item.offset, item_offset);
            write_u32(&mut item.size, item_size);
            result_items.push(item);
            data_chunks.push(data);
        }

        let mut result_data = Vec::with_capacity(total_data_len);
        for data in data_chunks.into_iter().rev() {
            result_data.extend_from_slice(&data);
        }

        Ok((result_items, result_data))
    }
}

impl Default for LeafBuilder {
    fn default() -> Self {
        Self::new()
    }
}
