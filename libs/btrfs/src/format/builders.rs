//! Builder patterns for complex btrfs structures.

use uuid::Uuid;

use super::accessors::{write_disk_key, write_u16, write_u32, write_u64};
use super::layout::{
    BTRFS_CHUNK_TREE_OBJECTID, BTRFS_DEFAULT_NODESIZE_U64, BTRFS_FIRST_CHUNK_TREE_OBJECTID,
};
use super::structures::{
    AsBytes as _, BTRFS_UUID_SIZE, BtrfsDevExtent, BtrfsDirItem, BtrfsDiskKey, BtrfsInodeItem,
    BtrfsInodeRef, BtrfsRootItem, BtrfsTimespec,
};
use crate::error::{BtrfsError, Result};

/// Directory item file type value.
pub(super) const BTRFS_FT_DIR: u8 = 2;

/// Directory inode mode bit.
const S_IFDIR: u32 = 0o040_000;

/// Builder for `BtrfsRootItem` to simplify complex initialization.
#[derive(Debug)]
pub struct RootItemBuilder {
    generation: u64,
    bytenr: u64,
    bytes_used: u64,
    root_dirid: u64,
    mode: u32,
    flags: u64,
    uuid: Option<[u8; BTRFS_UUID_SIZE]>,
    ctime: Option<u64>,
    otime: Option<u64>,
}

impl RootItemBuilder {
    /// Create a new root item builder.
    #[must_use]
    pub fn new() -> Self {
        Self {
            generation: 1,
            bytenr: 0,
            bytes_used: BTRFS_DEFAULT_NODESIZE_U64,
            root_dirid: 0,
            mode: S_IFDIR | 0o755,
            flags: 0,
            uuid: None,
            ctime: None,
            otime: None,
        }
    }

    /// Set the generation.
    #[must_use]
    pub fn generation(mut self, value: u64) -> Self {
        self.generation = value;
        self
    }

    /// Set the bytenr (block number).
    #[must_use]
    pub fn bytenr(mut self, bytenr: u64) -> Self {
        self.bytenr = bytenr;
        self
    }

    /// Set the root directory ID.
    #[must_use]
    pub fn root_dirid(mut self, dirid: u64) -> Self {
        self.root_dirid = dirid;
        self
    }

    /// Set the flags.
    #[must_use]
    pub fn flags(mut self, flags: u64) -> Self {
        self.flags = flags;
        self
    }

    /// Set the UUID.
    #[must_use]
    pub fn uuid(mut self, uuid: &Uuid) -> Self {
        let mut bytes = [0_u8; BTRFS_UUID_SIZE];
        bytes.copy_from_slice(uuid.as_bytes());
        self.uuid = Some(bytes);
        self
    }

    /// Set creation time.
    #[must_use]
    pub fn ctime(mut self, ctime: u64) -> Self {
        self.ctime = Some(ctime);
        self
    }

    /// Set origin time.
    #[must_use]
    pub fn otime(mut self, otime: u64) -> Self {
        self.otime = Some(otime);
        self
    }

    /// Build the `BtrfsRootItem`.
    #[must_use]
    pub fn build(self) -> BtrfsRootItem {
        let mut ri = BtrfsRootItem {
            inode: BtrfsInodeItem {
                generation: [0; 8],
                transid: [0; 8],
                size: [0; 8],
                nbytes: [0; 8],
                block_group: [0; 8],
                nlink: [0; 4],
                uid: [0; 4],
                gid: [0; 4],
                mode: [0; 4],
                rdev: [0; 8],
                flags: [0; 8],
                sequence: [0; 8],
                reserved: [0; 32],
                atime: BtrfsTimespec {
                    sec: [0; 8],
                    nsec: [0; 4],
                },
                ctime: BtrfsTimespec {
                    sec: [0; 8],
                    nsec: [0; 4],
                },
                mtime: BtrfsTimespec {
                    sec: [0; 8],
                    nsec: [0; 4],
                },
                otime: BtrfsTimespec {
                    sec: [0; 8],
                    nsec: [0; 4],
                },
            },
            generation: [0; 8],
            root_dirid: [0; 8],
            bytenr: [0; 8],
            byte_limit: [0; 8],
            bytes_used: [0; 8],
            last_snapshot: [0; 8],
            flags: [0; 8],
            refs: [0; 4],
            drop_progress: BtrfsDiskKey {
                objectid: [0; 8],
                type_: 0,
                offset: [0; 8],
            },
            drop_level: 0,
            level: 0,
            generation_v2: [0; 8],
            uuid: [0; BTRFS_UUID_SIZE],
            parent_uuid: [0; BTRFS_UUID_SIZE],
            received_uuid: [0; BTRFS_UUID_SIZE],
            ctransid: [0; 8],
            otransid: [0; 8],
            stransid: [0; 8],
            rtransid: [0; 8],
            ctime: BtrfsTimespec {
                sec: [0; 8],
                nsec: [0; 4],
            },
            otime: BtrfsTimespec {
                sec: [0; 8],
                nsec: [0; 4],
            },
            stime: BtrfsTimespec {
                sec: [0; 8],
                nsec: [0; 4],
            },
            rtime: BtrfsTimespec {
                sec: [0; 8],
                nsec: [0; 4],
            },
            reserved: [0; 64],
        };

        // Set inode fields
        write_u64(&mut ri.inode.generation, 1);
        write_u64(&mut ri.inode.size, 3);
        write_u64(&mut ri.inode.nbytes, BTRFS_DEFAULT_NODESIZE_U64);
        write_u32(&mut ri.inode.nlink, 1);
        write_u32(&mut ri.inode.mode, self.mode);

        // Set root item fields
        write_u64(&mut ri.generation, self.generation);
        write_u64(&mut ri.generation_v2, self.generation);
        write_u64(&mut ri.bytenr, self.bytenr);
        write_u64(&mut ri.bytes_used, self.bytes_used);
        write_u32(&mut ri.refs, 1);
        write_u64(&mut ri.root_dirid, self.root_dirid);
        write_u64(&mut ri.inode.flags, self.flags);

        if let Some(uuid) = self.uuid {
            ri.uuid.copy_from_slice(&uuid);
        }

        if let Some(ctime) = self.ctime {
            write_u64(&mut ri.ctime.sec, ctime);
        }

        if let Some(otime) = self.otime {
            write_u64(&mut ri.otime.sec, otime);
        }

        ri
    }
}

impl Default for RootItemBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for `BtrfsInodeItem`.
#[derive(Debug)]
pub struct InodeItemBuilder {
    generation: u64,
    nlink: u32,
    mode: u32,
    nbytes: u64,
    atime: u64,
    ctime: u64,
    mtime: u64,
    otime: u64,
}

impl InodeItemBuilder {
    /// Create a new inode item builder with default directory settings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            generation: 1,
            nlink: 1,
            mode: S_IFDIR | 0o755,
            nbytes: BTRFS_DEFAULT_NODESIZE_U64,
            atime: 0,
            ctime: 0,
            mtime: 0,
            otime: 0,
        }
    }

    /// Set the generation.
    #[must_use]
    pub fn generation(mut self, value: u64) -> Self {
        self.generation = value;
        self
    }

    /// Set timestamps (all four at once).
    #[must_use]
    pub fn timestamps(mut self, time: u64) -> Self {
        self.atime = time;
        self.ctime = time;
        self.mtime = time;
        self.otime = time;
        self
    }

    /// Build the `BtrfsInodeItem`.
    #[must_use]
    pub fn build(self) -> BtrfsInodeItem {
        let mut inode = BtrfsInodeItem {
            generation: [0; 8],
            transid: [0; 8],
            size: [0; 8],
            nbytes: [0; 8],
            block_group: [0; 8],
            nlink: [0; 4],
            uid: [0; 4],
            gid: [0; 4],
            mode: [0; 4],
            rdev: [0; 8],
            flags: [0; 8],
            sequence: [0; 8],
            reserved: [0; 32],
            atime: BtrfsTimespec {
                sec: [0; 8],
                nsec: [0; 4],
            },
            ctime: BtrfsTimespec {
                sec: [0; 8],
                nsec: [0; 4],
            },
            mtime: BtrfsTimespec {
                sec: [0; 8],
                nsec: [0; 4],
            },
            otime: BtrfsTimespec {
                sec: [0; 8],
                nsec: [0; 4],
            },
        };

        write_u64(&mut inode.generation, self.generation);
        write_u32(&mut inode.nlink, self.nlink);
        write_u32(&mut inode.mode, self.mode);
        write_u64(&mut inode.nbytes, self.nbytes);
        write_u64(&mut inode.atime.sec, self.atime);
        write_u64(&mut inode.ctime.sec, self.ctime);
        write_u64(&mut inode.mtime.sec, self.mtime);
        write_u64(&mut inode.otime.sec, self.otime);

        inode
    }
}

impl Default for InodeItemBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a `BtrfsInodeRef` structure.
pub fn build_inode_ref(name: &[u8]) -> Result<Vec<u8>> {
    let mut iref = BtrfsInodeRef {
        index: [0; 8],
        name_len: [0; 2],
    };
    let name_len = u16::try_from(name.len())
        .map_err(|_error| BtrfsError::Mkfs("inode reference name is too long".to_owned()))?;
    write_u16(&mut iref.name_len, name_len);
    let mut data = iref.to_vec();
    data.extend_from_slice(name);
    Ok(data)
}

/// Build a `BtrfsDirItem` structure.
pub fn build_dir_item(
    objectid: u64,
    type_key: u8,
    name: &[u8],
    generation: u64,
) -> Result<Vec<u8>> {
    let mut di = BtrfsDirItem {
        location: BtrfsDiskKey {
            objectid: [0; 8],
            type_: 0,
            offset: [0; 8],
        },
        transid: [0; 8],
        data_len: [0; 2],
        name_len: [0; 2],
        type_: BTRFS_FT_DIR,
    };
    write_disk_key(&mut di.location, objectid, type_key, u64::MAX);
    write_u64(&mut di.transid, generation);
    let name_len = u16::try_from(name.len())
        .map_err(|_error| BtrfsError::Mkfs("directory item name is too long".to_owned()))?;
    write_u16(&mut di.name_len, name_len);
    let mut data = di.to_vec();
    data.extend_from_slice(name);
    Ok(data)
}

/// Build a `BtrfsDevExtent` structure.
pub fn build_dev_extent(chunk_offset: u64, length: u64, chunk_tree_uuid: &uuid::Uuid) -> Vec<u8> {
    let mut de = BtrfsDevExtent {
        chunk_tree: [0; 8],
        chunk_objectid: [0; 8],
        chunk_offset: [0; 8],
        length: [0; 8],
        chunk_tree_uuid: [0; BTRFS_UUID_SIZE],
    };
    write_u64(&mut de.chunk_tree, BTRFS_CHUNK_TREE_OBJECTID);
    write_u64(&mut de.chunk_objectid, BTRFS_FIRST_CHUNK_TREE_OBJECTID);
    write_u64(&mut de.chunk_offset, chunk_offset);
    write_u64(&mut de.length, length);
    de.chunk_tree_uuid
        .copy_from_slice(chunk_tree_uuid.as_bytes());
    de.to_vec()
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    fn test_uuid() -> Uuid {
        Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_DEAD_BEEF)
    }

    fn le64(data: &[u8], at: usize) -> u64 {
        u64::from_le_bytes(
            data.get(at..at.saturating_add(8))
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
    fn root_item_defaults_match_mkfs_root_directory() {
        // ARRANGE
        let builder = RootItemBuilder::new();

        // ACT
        let item = builder.build();

        // ASSERT
        assert_eq!(u64::from_le_bytes(item.inode.generation), 1);
        assert_eq!(u64::from_le_bytes(item.inode.size), 3);
        assert_eq!(u64::from_le_bytes(item.inode.nbytes), 16_384);
        assert_eq!(u32::from_le_bytes(item.inode.nlink), 1);
        assert_eq!(u32::from_le_bytes(item.inode.mode), 0o040_755);
        assert_eq!(u64::from_le_bytes(item.generation), 1);
        assert_eq!(u64::from_le_bytes(item.generation_v2), 1);
        assert_eq!(u64::from_le_bytes(item.bytes_used), 16_384);
        assert_eq!(u32::from_le_bytes(item.refs), 1);
        assert_eq!(u64::from_le_bytes(item.bytenr), 0);
        assert_eq!(item.uuid, [0; 16]);
    }

    #[test]
    fn root_item_builder_applies_all_setters() {
        // ARRANGE
        let uuid = test_uuid();
        let builder = RootItemBuilder::new()
            .generation(9)
            .bytenr(0x20_0000)
            .root_dirid(256)
            .flags(0x40)
            .uuid(&uuid)
            .ctime(111)
            .otime(222);

        // ACT
        let item = builder.build();

        // ASSERT
        assert_eq!(u64::from_le_bytes(item.generation), 9);
        assert_eq!(u64::from_le_bytes(item.generation_v2), 9);
        assert_eq!(u64::from_le_bytes(item.bytenr), 0x20_0000);
        assert_eq!(u64::from_le_bytes(item.root_dirid), 256);
        assert_eq!(u64::from_le_bytes(item.inode.flags), 0x40);
        assert_eq!(item.uuid, *uuid.as_bytes());
        assert_eq!(u64::from_le_bytes(item.ctime.sec), 111);
        assert_eq!(u64::from_le_bytes(item.otime.sec), 222);
    }

    #[test]
    fn inode_item_defaults_are_directory_inode() {
        // ARRANGE
        let builder = InodeItemBuilder::new();

        // ACT
        let item = builder.build();

        // ASSERT
        assert_eq!(u64::from_le_bytes(item.generation), 1);
        assert_eq!(u32::from_le_bytes(item.nlink), 1);
        assert_eq!(u32::from_le_bytes(item.mode), 0o040_755);
        assert_eq!(u64::from_le_bytes(item.nbytes), 16_384);
        assert_eq!(u64::from_le_bytes(item.atime.sec), 0);
    }

    #[test]
    fn inode_item_timestamps_apply_to_all_four_times() {
        // ARRANGE
        let builder = InodeItemBuilder::new().generation(5).timestamps(777);

        // ACT
        let item = builder.build();

        // ASSERT
        assert_eq!(u64::from_le_bytes(item.generation), 5);
        assert_eq!(u64::from_le_bytes(item.atime.sec), 777);
        assert_eq!(u64::from_le_bytes(item.ctime.sec), 777);
        assert_eq!(u64::from_le_bytes(item.mtime.sec), 777);
        assert_eq!(u64::from_le_bytes(item.otime.sec), 777);
    }

    #[test]
    fn inode_ref_appends_name_after_header() {
        // ARRANGE
        let name: &[u8] = b"..";

        // ACT
        let data = build_inode_ref(name).unwrap();

        // ASSERT
        assert_eq!(data.len(), 12);
        assert_eq!(le64(&data, 0), 0);
        assert_eq!(le16(&data, 8), 2);
        assert_eq!(data.get(10..12), Some(b"..".as_slice()));
    }

    #[test]
    fn inode_ref_rejects_names_longer_than_u16() {
        // ARRANGE
        let name = vec![b'a'; 65_536];

        // ACT
        let result = build_inode_ref(&name);

        // ASSERT
        assert!(matches!(result, Err(BtrfsError::Mkfs(_))));
    }

    #[test]
    fn dir_item_encodes_location_transid_and_name() {
        // ARRANGE
        let name: &[u8] = b"default";

        // ACT
        let data = build_dir_item(5, 132, name, 7).unwrap();

        // ASSERT
        assert_eq!(data.len(), 37);
        assert_eq!(le64(&data, 0), 5);
        assert_eq!(data.get(8), Some(&132));
        assert_eq!(le64(&data, 9), u64::MAX);
        assert_eq!(le64(&data, 17), 7);
        assert_eq!(le16(&data, 25), 0);
        assert_eq!(le16(&data, 27), 7);
        assert_eq!(data.get(29), Some(&2));
        assert_eq!(data.get(30..37), Some(b"default".as_slice()));
    }

    #[test]
    fn dir_item_rejects_names_longer_than_u16() {
        // ARRANGE
        let name = vec![b'n'; 65_536];

        // ACT
        let result = build_dir_item(1, 1, &name, 1);

        // ASSERT
        assert!(matches!(result, Err(BtrfsError::Mkfs(_))));
    }

    #[test]
    fn dev_extent_encodes_chunk_tree_objectid_and_uuid() {
        // ARRANGE
        let chunk_uuid = test_uuid();

        // ACT
        let data = build_dev_extent(0x10_0000, 0x40_0000, &chunk_uuid);

        // ASSERT
        assert_eq!(data.len(), 48);
        assert_eq!(le64(&data, 0), BTRFS_CHUNK_TREE_OBJECTID);
        assert_eq!(le64(&data, 8), BTRFS_FIRST_CHUNK_TREE_OBJECTID);
        assert_eq!(le64(&data, 16), 0x10_0000);
        assert_eq!(le64(&data, 24), 0x40_0000);
        assert_eq!(data.get(32..48), Some(chunk_uuid.as_bytes().as_slice()));
    }
}
