//! Directory block construction with sorted dirent arrays.

use crate::checked::{u16_from_usize, write_byte, write_bytes};

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
pub fn data_size(entries: &[DirEntry]) -> usize {
    let block_size = 4096_usize;
    let mut size = 0_usize;

    for entry in entries {
        let entry_size = DIRENT_SIZE.saturating_add(entry.name.len());
        if size.rem_euclid(block_size).saturating_add(entry_size) > block_size {
            size = size.div_ceil(block_size).saturating_mul(block_size);
        }
        size = size.saturating_add(entry_size);
    }

    size
}

/// Serialize directory entries into EROFS directory blocks.
pub fn serialize_entries(entries: &[DirEntry]) -> Vec<u8> {
    let block_size = 4096_usize;
    let total_size = data_size(entries);
    let mut buf = vec![0_u8; total_size];
    let mut block_start = 0_usize;
    let mut start_index = 0_usize;

    while start_index < entries.len() {
        let end_index = block_end(entries, start_index, block_size);
        let mut dirent_offset = 0_usize;
        let mut name_offset = end_index
            .saturating_sub(start_index)
            .saturating_mul(DIRENT_SIZE);

        for entry in entries
            .iter()
            .skip(start_index)
            .take(end_index.saturating_sub(start_index))
        {
            let dirent_start = block_start.saturating_add(dirent_offset);
            let name_start = block_start.saturating_add(name_offset);
            let name_offset_u16 = u16_from_usize(name_offset).unwrap_or_default();

            let wrote_all = write_bytes(&mut buf, dirent_start, &entry.nid.to_le_bytes())
                && write_bytes(
                    &mut buf,
                    dirent_start.saturating_add(8),
                    &name_offset_u16.to_le_bytes(),
                )
                && write_byte(&mut buf, dirent_start.saturating_add(10), entry.file_type)
                && write_byte(&mut buf, dirent_start.saturating_add(11), 0)
                && write_bytes(&mut buf, name_start, &entry.name);

            assert!(
                wrote_all,
                "directory serialization must fit the precomputed output buffer"
            );

            dirent_offset = dirent_offset.saturating_add(DIRENT_SIZE);
            name_offset = name_offset.saturating_add(entry.name.len());
        }

        let block_advance = if end_index == entries.len() {
            total_size.saturating_sub(block_start)
        } else {
            block_size
        };
        block_start = block_start.saturating_add(block_advance);
        start_index = end_index;
    }

    buf
}

/// Determine how many entries fit in the current block starting from `start`.
fn block_end(entries: &[DirEntry], start: usize, bs: usize) -> usize {
    let mut used = 0_usize;
    let mut end = start;
    while let Some(entry) = entries.get(end) {
        let entry_size = DIRENT_SIZE.saturating_add(entry.name.len());
        if used.saturating_add(entry_size) > bs {
            break;
        }
        used = used.saturating_add(entry_size);
        end = end.saturating_add(1);
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
        let size = data_size(&entries);

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
        let size = data_size(&entries);

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
        let data = serialize_entries(&entries);

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
        let data = serialize_entries(&entries);

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
