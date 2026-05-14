//! Directory block construction with sorted dirent arrays.

pub const EROFS_FT_REG_FILE: u8 = 1;
pub const EROFS_FT_DIR: u8 = 2;
pub const EROFS_FT_SYMLINK: u8 = 7;
pub const EROFS_NAME_LEN: usize = 255;
pub const DIRENT_SIZE: usize = 12;

/// A single directory entry before serialization.
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: Vec<u8>,
    pub nid: u64,
    pub file_type: u8,
}

/// Compute the actual directory data size in bytes (dirent array + packed names).
pub fn dir_data_size(entries: &[DirEntry]) -> usize {
    let bs = crate::BLOCK_SIZE as usize;
    let mut size = 0usize;

    for entry in entries {
        let len = DIRENT_SIZE + entry.name.len();
        if size % bs + len > bs {
            size = size.div_ceil(bs) * bs;
        }
        size += len;
    }

    size
}

/// Serialize directory entries into EROFS directory blocks.
pub fn serialize_dir_entries(entries: &[DirEntry]) -> Vec<u8> {
    let bs = crate::BLOCK_SIZE as usize;
    let total_size = dir_data_size(entries);
    let mut buf = vec![0u8; total_size];
    let mut block_start = 0usize;
    let mut idx = 0usize;

    while idx < entries.len() {
        let block_end = block_end(entries, idx, bs);
        let mut dirent_offset = 0usize;
        let mut name_offset = (block_end - idx) * DIRENT_SIZE;

        for entry in &entries[idx..block_end] {
            let d = block_start + dirent_offset;
            let name_start = block_start + name_offset;
            let name_end = name_start + entry.name.len();

            buf[d..d + 8].copy_from_slice(&entry.nid.to_le_bytes());
            buf[d + 8..d + 10].copy_from_slice(&(name_offset as u16).to_le_bytes());
            buf[d + 10] = entry.file_type;
            buf[d + 11] = 0;
            buf[name_start..name_end].copy_from_slice(&entry.name);

            dirent_offset += DIRENT_SIZE;
            name_offset += entry.name.len();
        }

        block_start += if block_end == entries.len() {
            total_size - block_start
        } else {
            bs
        };
        idx = block_end;
    }

    buf
}

/// Determine how many entries fit in the current block starting from `start`.
fn block_end(entries: &[DirEntry], start: usize, bs: usize) -> usize {
    let mut used = 0usize;
    let mut end = start;
    while end < entries.len() {
        let len = DIRENT_SIZE + entries[end].name.len();
        if used + len > bs {
            break;
        }
        used += len;
        end += 1;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_data_size_exact() {
        // ARRANGE
        let entries = vec![
            DirEntry {
                name: b".".to_vec(),
                nid: 0,
                file_type: EROFS_FT_DIR,
            },
            DirEntry {
                name: b"..".to_vec(),
                nid: 0,
                file_type: EROFS_FT_DIR,
            },
            DirEntry {
                name: b"hello.txt".to_vec(),
                nid: 1,
                file_type: EROFS_FT_REG_FILE,
            },
        ];

        // ACT
        let size = dir_data_size(&entries);

        // ASSERT
        assert_eq!(size, 3 * 12 + 1 + 2 + 9);
    }

    #[test]
    fn dir_data_size_includes_block_padding() {
        // ARRANGE
        let mut entries = vec![
            DirEntry {
                name: b".".to_vec(),
                nid: 36,
                file_type: EROFS_FT_DIR,
            },
            DirEntry {
                name: b"..".to_vec(),
                nid: 36,
                file_type: EROFS_FT_DIR,
            },
        ];
        entries.extend((0..400u16).map(|i| DirEntry {
            name: format!("file_{i:03}.txt").into_bytes(),
            nid: u64::from(i) + 40,
            file_type: EROFS_FT_REG_FILE,
        }));
        let packed =
            entries.len() * DIRENT_SIZE + entries.iter().map(|e| e.name.len()).sum::<usize>();

        // ACT
        let size = dir_data_size(&entries);

        // ASSERT
        assert!(size > packed);
        assert!(size > crate::BLOCK_SIZE as usize);
    }

    #[test]
    fn serialize_produces_correct_layout() {
        // ARRANGE
        let entries = vec![
            DirEntry {
                name: b".".to_vec(),
                nid: 36,
                file_type: EROFS_FT_DIR,
            },
            DirEntry {
                name: b"..".to_vec(),
                nid: 36,
                file_type: EROFS_FT_DIR,
            },
            DirEntry {
                name: b"hello".to_vec(),
                nid: 40,
                file_type: EROFS_FT_REG_FILE,
            },
        ];

        // ACT
        let data = serialize_dir_entries(&entries);

        // ASSERT
        assert_eq!(data.len(), 3 * 12 + 1 + 2 + 5);

        let nameoff0 = u16::from_le_bytes(data[8..10].try_into().expect("2 bytes"));
        assert_eq!(nameoff0 as usize, 3 * DIRENT_SIZE);

        assert_eq!(data[10], EROFS_FT_DIR);
    }

    #[test]
    fn serialize_resets_name_offsets_per_block() {
        // ARRANGE
        let mut entries = vec![
            DirEntry {
                name: b".".to_vec(),
                nid: 36,
                file_type: EROFS_FT_DIR,
            },
            DirEntry {
                name: b"..".to_vec(),
                nid: 36,
                file_type: EROFS_FT_DIR,
            },
        ];
        entries.extend((0..400u16).map(|i| DirEntry {
            name: format!("file_{i:03}.txt").into_bytes(),
            nid: u64::from(i) + 40,
            file_type: EROFS_FT_REG_FILE,
        }));

        // ACT
        let data = serialize_dir_entries(&entries);

        // ASSERT
        let second_block = crate::BLOCK_SIZE as usize;
        let nameoff = u16::from_le_bytes(
            data[second_block + 8..second_block + 10]
                .try_into()
                .expect("2 bytes"),
        );
        assert!(data.len() > second_block);
        assert!(usize::from(nameoff) < crate::BLOCK_SIZE as usize);
    }

    #[test]
    fn lexicographic_sort_required() {
        // ARRANGE
        let mut entries = vec![
            DirEntry {
                name: b"..".to_vec(),
                nid: 0,
                file_type: EROFS_FT_DIR,
            },
            DirEntry {
                name: b".".to_vec(),
                nid: 0,
                file_type: EROFS_FT_DIR,
            },
            DirEntry {
                name: b"b".to_vec(),
                nid: 2,
                file_type: EROFS_FT_REG_FILE,
            },
            DirEntry {
                name: b"a".to_vec(),
                nid: 1,
                file_type: EROFS_FT_REG_FILE,
            },
        ];

        // ACT
        entries.sort_by(|a, b| a.name.cmp(&b.name));

        // ASSERT
        assert_eq!(entries[0].name, b".");
        assert_eq!(entries[1].name, b"..");
        assert_eq!(entries[2].name, b"a");
        assert_eq!(entries[3].name, b"b");
    }
}
