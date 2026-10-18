//! FAT directory data construction.

use std::io::Write;

use crate::error::Result;
use crate::name::{self, entry};
use crate::tree::{DirIndex, Kind};
use crate::types::{
    ATTR_ARCHIVE, ATTR_DIRECTORY, ClusterMap, FileMeta, ROOT_CLUSTER, VOLUME_LABEL,
};

/// Index of the root directory in a `DirIndex`.
const ROOT_DIR_INDEX: usize = 0;

/// Predicted byte size of every directory, including the root volume label.
pub(crate) fn sizes<'a>(
    tree: &DirIndex<'a>,
    files: &'a [FileMeta<'a>],
    aliases: &[Vec<[u8; name::short::NAME_LEN]>],
) -> Vec<u64> {
    (0..tree.len())
        .map(|dir_index| size_of(tree, files, aliases, dir_index))
        .collect()
}

/// Builds the entry bytes of one directory.
pub(crate) fn build_data<'a>(
    files: &'a [FileMeta<'a>],
    tree: &DirIndex<'a>,
    map: &ClusterMap,
    aliases: &[Vec<[u8; name::short::NAME_LEN]>],
    dir_index: usize,
) -> Vec<u8> {
    let own = *map.dir_starts.get(dir_index).unwrap_or(&ROOT_CLUSTER);
    let parent = parent_cluster(tree, map, dir_index);
    let mut data = Vec::new();
    if dir_index == ROOT_DIR_INDEX {
        data.extend_from_slice(&entry::label_entry(&VOLUME_LABEL));
    } else {
        data.extend_from_slice(&entry::dot_entries(own, parent));
    }
    let shorts = aliases.get(dir_index).map_or(&[][..], Vec::as_slice);
    for ((name, kind), alias) in tree.entries(files, dir_index).zip(shorts) {
        let (attr, cluster, size) = match kind {
            Kind::Directory(index) => (
                ATTR_DIRECTORY,
                *map.dir_starts.get(index).unwrap_or(&ROOT_CLUSTER),
                0,
            ),
            Kind::File(index) => (
                ATTR_ARCHIVE,
                *map.file_starts.get(index).unwrap_or(&ROOT_CLUSTER),
                u32::try_from(files.get(index).map_or(0, |file| file.size)).unwrap_or(u32::MAX),
            ),
        };
        entry::append_entry(&mut data, name, alias, attr, cluster, size);
    }

    data
}

pub(crate) fn write_zeros<W: Write>(writer: &mut W, count: u64) -> Result<()> {
    const ZERO_BUF: [u8; 8192] = [0_u8; 8192];
    let buf_len = u64::try_from(ZERO_BUF.len()).unwrap_or(u64::MAX);
    let mut rem = count;
    while rem > 0 {
        let chunk = rem.min(buf_len);
        let n = usize::try_from(chunk).unwrap_or(ZERO_BUF.len());
        writer.write_all(ZERO_BUF.get(..n).unwrap_or(&[]))?;
        rem = rem.saturating_sub(chunk);
    }

    Ok(())
}

fn parent_cluster(tree: &DirIndex<'_>, map: &ClusterMap, dir_index: usize) -> u32 {
    let parent = tree.parent_of(dir_index);
    if parent == ROOT_DIR_INDEX {
        return 0;
    }

    *map.dir_starts.get(parent).unwrap_or(&ROOT_CLUSTER)
}

fn size_of<'a>(
    tree: &DirIndex<'a>,
    files: &'a [FileMeta<'a>],
    aliases: &[Vec<[u8; name::short::NAME_LEN]>],
    dir_index: usize,
) -> u64 {
    let shorts = aliases.get(dir_index).map_or(&[][..], Vec::as_slice);
    let entries = tree
        .entries(files, dir_index)
        .zip(shorts)
        .map(|((name, _), alias)| entry::entry_bytes_len(name, alias))
        .sum::<u64>();
    let header = if dir_index == ROOT_DIR_INDEX {
        entry::ENTRY_LEN
    } else {
        entry::ENTRY_LEN.saturating_mul(2)
    };

    header.saturating_add(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::name::short::ShortNames;

    fn aliases_for<'a>(
        tree: &DirIndex<'a>,
        files: &'a [FileMeta<'a>],
    ) -> Vec<Vec<[u8; name::short::NAME_LEN]>> {
        (0..tree.len())
            .map(|dir_index| {
                let mut allocator = ShortNames::new();
                tree.entries(files, dir_index)
                    .map(|(name, _)| allocator.assign(name).expect("alias must be assigned"))
                    .collect()
            })
            .collect()
    }

    fn sample<'a>(
        files: &'a [FileMeta<'a>],
    ) -> (
        DirIndex<'a>,
        Vec<Vec<[u8; name::short::NAME_LEN]>>,
        Vec<u64>,
    ) {
        let tree = DirIndex::collect(files);
        let aliases = aliases_for(&tree, files);
        let sizes = sizes(&tree, files, &aliases);

        (tree, aliases, sizes)
    }

    fn sample_map() -> ClusterMap {
        ClusterMap {
            dir_starts: vec![2, 5, 6],
            dir_counts: vec![3, 1, 1],
            file_starts: vec![7, 8, 9],
            file_counts: vec![1, 1, 1],
            file_sizes: vec![11, 6, 2],
        }
    }

    #[test]
    fn sizes_match_the_emitted_bytes() {
        // ARRANGE
        let files = &[
            FileMeta::new("EFI/BOOT/BOOTAA64.EFI", 11),
            FileMeta::new("config.txt", 6),
            FileMeta::new("act-led.dtbo", 2),
        ];
        let (tree, aliases, sizes) = sample(files);
        let map = sample_map();

        // ACT / ASSERT
        for dir_index in 0..tree.len() {
            let data = build_data(files, &tree, &map, &aliases, dir_index);
            assert_eq!(
                u64::try_from(data.len()).unwrap_or(u64::MAX),
                *sizes.get(dir_index).unwrap_or(&0),
                "size of {}",
                tree.path(dir_index)
            );
        }
    }

    #[test]
    fn root_carries_the_volume_label() {
        // ARRANGE
        let files = &[FileMeta::new("config.txt", 6)];
        let (tree, aliases, sizes) = sample(files);
        let map = ClusterMap {
            dir_starts: vec![2],
            dir_counts: vec![1],
            file_starts: vec![3],
            file_counts: vec![1],
            file_sizes: vec![6],
        };

        // ACT
        let data = build_data(files, &tree, &map, &aliases, 0);

        // ASSERT
        assert_eq!(
            data.get(..11),
            Some(&VOLUME_LABEL[..]),
            "the label entry must come first"
        );
        assert_eq!(
            data.get(11),
            Some(&crate::types::ATTR_VOLUME_ID),
            "label attribute"
        );
        assert_eq!(
            u64::try_from(data.len()).unwrap_or(u64::MAX),
            *sizes.first().unwrap_or(&0)
        );
    }

    #[test]
    fn nested_directories_emit_dot_entries_first() {
        // ARRANGE
        let files = &[FileMeta::new("EFI/BOOT/BOOTAA64.EFI", 11)];
        let (tree, aliases, _) = sample(files);
        let map = ClusterMap {
            dir_starts: vec![2, 3, 4],
            dir_counts: vec![1, 1, 1],
            file_starts: vec![5],
            file_counts: vec![1],
            file_sizes: vec![11],
        };

        // ACT
        let data = build_data(files, &tree, &map, &aliases, 1);

        // ASSERT
        assert_eq!(data.get(..3), Some(&b".  "[..]), "`.` first");
        assert_eq!(data.get(32..35), Some(&b".. "[..]), "`..` second");
        assert_eq!(
            data.get(64..70),
            Some(&b"BOOT  "[..]),
            "the subdirectory follows"
        );
    }
}
