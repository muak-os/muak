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

#[cfg(test)]
mod tests {
    use super::*;

    const LEAF_CAPACITY: u32 = 16_283; // BTRFS_DEFAULT_NODESIZE - BTRFS_LEAF_DATA_OFFSET

    fn item_key(item: &BtrfsItem) -> (u64, u8, u64) {
        (
            u64::from_le_bytes(item.key.objectid),
            item.key.type_,
            u64::from_le_bytes(item.key.offset),
        )
    }

    #[test]
    fn empty_leaf_builds_no_items() {
        // ARRANGE
        let builder = LeafBuilder::new();

        // ACT
        let (items, data) = builder.build().unwrap();

        // ASSERT
        assert!(items.is_empty());
        assert!(data.is_empty());
    }

    #[test]
    fn single_item_places_data_at_leaf_tail() {
        // ARRANGE
        let mut builder = LeafBuilder::new();
        builder.add_item(7, 84, 9, vec![1, 2, 3]);

        // ACT
        let (items, data) = builder.build().unwrap();

        // ASSERT
        let item = items.first().unwrap();
        assert_eq!(item_key(item), (7, 84, 9));
        assert_eq!(u32::from_le_bytes(item.size), 3);
        assert_eq!(u32::from_le_bytes(item.offset), LEAF_CAPACITY - 3);
        assert_eq!(data, vec![1, 2, 3]);
    }

    #[test]
    fn items_are_sorted_by_objectid_type_offset() {
        // ARRANGE
        let mut builder = LeafBuilder::new();
        builder.add_item(9, 1, 0, vec![1]);
        builder.add_item(1, 1, 0, vec![2]);
        builder.add_item(5, 84, 7, vec![3]);
        builder.add_item(5, 12, 9, vec![4]);
        builder.add_item(5, 84, 3, vec![5]);

        // ACT
        let (items, _) = builder.build().unwrap();

        // ASSERT
        let keys: Vec<_> = items.iter().map(item_key).collect();
        assert_eq!(
            keys,
            vec![(1, 1, 0), (5, 12, 9), (5, 84, 3), (5, 84, 7), (9, 1, 0)]
        );
    }

    #[test]
    fn data_chunks_are_stacked_from_leaf_tail_backwards() {
        // ARRANGE
        let mut builder = LeafBuilder::new();
        builder.add_item(1, 1, 0, vec![0xAA]);
        builder.add_item(2, 1, 0, vec![0xBB, 0xCC]);

        // ACT
        let (items, data) = builder.build().unwrap();

        // ASSERT
        let first = items.first().unwrap();
        let second = items.get(1).unwrap();
        assert_eq!(u32::from_le_bytes(first.offset), LEAF_CAPACITY - 1);
        assert_eq!(u32::from_le_bytes(second.offset), LEAF_CAPACITY - 3);
        assert_eq!(data, vec![0xBB, 0xCC, 0xAA]);
    }

    #[test]
    fn build_rejects_data_larger_than_leaf_capacity() {
        // ARRANGE
        let mut builder = LeafBuilder::new();
        builder.add_item(1, 1, 0, vec![0_u8; 16_300]);

        // ACT
        let result = builder.build();

        // ASSERT
        assert!(matches!(result, Err(BtrfsError::Mkfs(_))));
    }
}
