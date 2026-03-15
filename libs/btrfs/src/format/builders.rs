//! Builder patterns for complex btrfs structures.

use uuid::Uuid;

use super::accessors::*;
use super::constants::*;
use super::structures::*;

/// Builder for BtrfsRootItem to simplify complex initialization.
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
    pub fn new() -> Self {
        Self {
            generation: 1,
            bytenr: 0,
            bytes_used: BTRFS_DEFAULT_NODESIZE as u64,
            root_dirid: 0,
            mode: S_IFDIR | 0o755,
            flags: 0,
            uuid: None,
            ctime: None,
            otime: None,
        }
    }

    /// Set the generation.
    pub fn generation(mut self, value: u64) -> Self {
        self.generation = value;
        self
    }

    /// Set the bytenr (block number).
    pub fn bytenr(mut self, bytenr: u64) -> Self {
        self.bytenr = bytenr;
        self
    }

    /// Set the root directory ID.
    pub fn root_dirid(mut self, dirid: u64) -> Self {
        self.root_dirid = dirid;
        self
    }

    /// Set the flags.
    pub fn flags(mut self, flags: u64) -> Self {
        self.flags = flags;
        self
    }

    /// Set the UUID.
    pub fn uuid(mut self, uuid: &Uuid) -> Self {
        let mut bytes = [0u8; BTRFS_UUID_SIZE];
        bytes.copy_from_slice(uuid.as_bytes());
        self.uuid = Some(bytes);
        self
    }

    /// Set creation time.
    pub fn ctime(mut self, ctime: u64) -> Self {
        self.ctime = Some(ctime);
        self
    }

    /// Set origin time.
    pub fn otime(mut self, otime: u64) -> Self {
        self.otime = Some(otime);
        self
    }

    /// Build the BtrfsRootItem.
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
        write_u64(&mut ri.inode.nbytes, BTRFS_DEFAULT_NODESIZE as u64);
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

/// Builder for BtrfsInodeItem.
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
    pub fn new() -> Self {
        Self {
            generation: 1,
            nlink: 1,
            mode: S_IFDIR | 0o755,
            nbytes: BTRFS_DEFAULT_NODESIZE as u64,
            atime: 0,
            ctime: 0,
            mtime: 0,
            otime: 0,
        }
    }

    /// Set the generation.
    pub fn generation(mut self, value: u64) -> Self {
        self.generation = value;
        self
    }

    /// Set timestamps (all four at once).
    pub fn timestamps(mut self, time: u64) -> Self {
        self.atime = time;
        self.ctime = time;
        self.mtime = time;
        self.otime = time;
        self
    }

    /// Build the BtrfsInodeItem.
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

/// Build a BtrfsInodeRef structure.
pub fn build_inode_ref(name: &[u8]) -> Vec<u8> {
    let mut iref = BtrfsInodeRef {
        index: [0; 8],
        name_len: [0; 2],
    };
    write_u16(&mut iref.name_len, name.len() as u16);
    let mut data = iref.to_vec();
    data.extend_from_slice(name);
    data
}

/// Build a BtrfsDirItem structure.
pub fn build_dir_item(objectid: u64, type_key: u8, name: &[u8], generation: u64) -> Vec<u8> {
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
    write_u16(&mut di.name_len, name.len() as u16);
    let mut data = di.to_vec();
    data.extend_from_slice(name);
    data
}

/// Build a BtrfsDevExtent structure.
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
