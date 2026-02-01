//! Node and leaf building utilities for Btrfs filesystem creation.

use super::accessors::*;
use super::constants::*;
use super::structures::*;

/// Builds Btrfs leaf nodes with properly sorted items.
#[derive(Debug)]
pub struct LeafBuilder {
    items: Vec<(u64, u8, u64, Vec<u8>)>,
}

impl LeafBuilder {
    /// Create a new empty leaf builder.
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Add an item to the leaf.
    pub fn add_item(&mut self, objectid: u64, type_: u8, offset: u64, data: Vec<u8>) {
        self.items.push((objectid, type_, offset, data));
    }

    /// Build the leaf items and data.
    ///
    /// Returns a tuple of (items, data) where items are the BtrfsItem structs
    /// and data is the concatenated item data.
    pub fn build(mut self) -> (Vec<BtrfsItem>, Vec<u8>) {
        self.items
            .sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));

        let mut result_items = Vec::new();
        let mut result_data = Vec::new();
        let mut current_offset = BTRFS_DEFAULT_NODESIZE as usize - BTRFS_LEAF_DATA_OFFSET;

        for (objectid, type_, offset, data) in self.items {
            current_offset -= data.len();

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
            write_u32(&mut item.offset, current_offset as u32);
            write_u32(&mut item.size, data.len() as u32);
            result_items.push(item);

            let mut new_data = data;
            new_data.extend_from_slice(&result_data);
            result_data = new_data;
        }

        (result_items, result_data)
    }
}

impl Default for LeafBuilder {
    fn default() -> Self {
        Self::new()
    }
}
