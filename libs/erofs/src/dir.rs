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
    entries.len() * DIRENT_SIZE + entries.iter().map(|e| e.name.len()).sum::<usize>()
}

/// Serialize directory entries into a contiguous byte buffer (no block padding).
pub fn serialize_dir_entries(entries: &[DirEntry]) -> Vec<u8> {
    let total_size = dir_data_size(entries);
    let mut buf = vec![0u8; total_size];
    let name_base = entries.len() * DIRENT_SIZE;
    let mut name_offset = name_base;

    for (i, entry) in entries.iter().enumerate() {
        let d = i * DIRENT_SIZE;
        buf[d..d + 8].copy_from_slice(&entry.nid.to_le_bytes());
        buf[d + 8..d + 10].copy_from_slice(&(name_offset as u16).to_le_bytes());
        buf[d + 10] = entry.file_type;
        buf[d + 11] = 0;

        let name_end = name_offset + entry.name.len();
        buf[name_offset..name_end].copy_from_slice(&entry.name);
        name_offset = name_end;
    }
    buf
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
