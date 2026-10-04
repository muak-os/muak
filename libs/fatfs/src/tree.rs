//! Single-pass directory-tree index for the FAT builder.

use std::collections::{HashMap, HashSet};

use crate::name;
use crate::types::FileMeta;

/// Structural index over the file set.
pub(crate) struct DirIndex<'a> {
    dirs: Vec<&'a str>,
    files_by_dir: Vec<Vec<usize>>,
    subdirs_by_dir: Vec<Vec<usize>>,
    parent: Vec<usize>,
}

impl core::fmt::Debug for DirIndex<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DirIndex")
            .field("dirs", &self.dirs.len())
            .field(
                "files",
                &self.files_by_dir.iter().map(Vec::len).sum::<usize>(),
            )
            .finish()
    }
}

impl<'a> DirIndex<'a> {
    /// Number of directories in the tree.
    pub(crate) fn len(&self) -> usize {
        self.dirs.len()
    }

    /// Directory path by index.
    pub(crate) fn path(&self, dir_index: usize) -> &'a str {
        self.dirs.get(dir_index).copied().unwrap_or("")
    }

    /// Index of a directory's parent (root maps to itself).
    pub(crate) fn parent_of(&self, dir_index: usize) -> usize {
        self.parent.get(dir_index).copied().unwrap_or(0)
    }

    /// File indices belonging to a directory, in input order.
    pub(crate) fn files(&self, dir_index: usize) -> &[usize] {
        self.files_by_dir.get(dir_index).map_or(&[], Vec::as_slice)
    }

    /// Subdirectory indices belonging to a directory, in sorted order.
    pub(crate) fn subdirs(&self, dir_index: usize) -> &[usize] {
        self.subdirs_by_dir
            .get(dir_index)
            .map_or(&[], Vec::as_slice)
    }

    /// Predicted directory byte sizes, derived analytically.
    /// set per child.
    pub(crate) fn sizes(&self, files: &[FileMeta<'a>]) -> Vec<u64> {
        self.dirs
            .iter()
            .enumerate()
            .map(|(dir_index, _)| self.dir_size(dir_index, files))
            .collect()
    }

    /// Builds the tree from the file set in one pass.
    pub(crate) fn collect(files: &[FileMeta<'a>]) -> Self {
        let mut dirs: Vec<&'a str> = vec![""];
        let mut prior: HashSet<&'a str> = HashSet::new();
        for file in files {
            register_ancestors(file.path, &mut dirs, &mut prior);
        }
        dirs.sort_by(|left, right| left.len().cmp(&right.len()).then(left.cmp(right)));
        let index: HashMap<&str, usize> =
            dirs.iter().enumerate().map(|(i, dir)| (*dir, i)).collect();

        let files_by_dir = group_files(files, &index, dirs.len());
        let (subdirs_by_dir, parent) = link_children(&dirs, &index);

        Self {
            dirs,
            files_by_dir,
            subdirs_by_dir,
            parent,
        }
    }

    /// File name component of a path (text after the last separator).
    pub(crate) fn name_of(path: &str) -> &str {
        match path.rfind('/') {
            Some(slash) => path.get(slash.wrapping_add(1)..).unwrap_or(""),
            None => path,
        }
    }

    fn dir_size(&self, dir_index: usize, files: &[FileMeta<'a>]) -> u64 {
        let dirs_bytes = self
            .subdirs(dir_index)
            .iter()
            .map(|&child| name::entry_bytes_len(Self::name_of(self.path(child))))
            .sum::<u64>();
        let files_bytes = self
            .files(dir_index)
            .iter()
            .filter_map(|&file_index| files.get(file_index))
            .map(|file| name::entry_bytes_len(Self::name_of(file.path)))
            .sum::<u64>();

        name::ENTRY_LEN
            .saturating_mul(2)
            .saturating_add(dirs_bytes)
            .saturating_add(files_bytes)
    }

    fn parent_path(path: &str) -> &str {
        match path.rfind('/') {
            Some(slash) => path.get(..slash).unwrap_or(""),
            None => "",
        }
    }
}

fn register_ancestors<'a>(path: &'a str, dirs: &mut Vec<&'a str>, prior: &mut HashSet<&'a str>) {
    let mut remaining = path;
    while let Some(slash) = remaining.rfind('/') {
        remaining = remaining.get(..slash).unwrap_or("");
        if prior.insert(remaining) {
            dirs.push(remaining);
        }
    }
}

fn group_files(
    files: &[FileMeta<'_>],
    index: &HashMap<&str, usize>,
    dir_count: usize,
) -> Vec<Vec<usize>> {
    let mut grouped = vec![Vec::new(); dir_count];
    for (file_index, file) in files.iter().enumerate() {
        let dir_index = index
            .get(DirIndex::parent_path(file.path))
            .copied()
            .unwrap_or(0);
        if let Some(bucket) = grouped.get_mut(dir_index) {
            bucket.push(file_index);
        }
    }

    grouped
}

fn link_children(dirs: &[&'_ str], index: &HashMap<&str, usize>) -> (Vec<Vec<usize>>, Vec<usize>) {
    let mut subdirs_by_dir = vec![Vec::new(); dirs.len()];
    let mut parent = vec![0; dirs.len()];
    for (dir_index, dir_path) in dirs.iter().enumerate() {
        let parent_index = index
            .get(DirIndex::parent_path(dir_path))
            .copied()
            .unwrap_or(0);
        if parent_index == dir_index {
            continue;
        }
        if let Some(slot) = parent.get_mut(dir_index) {
            *slot = parent_index;
        }
        if let Some(bucket) = subdirs_by_dir.get_mut(parent_index) {
            bucket.push(dir_index);
        }
    }

    (subdirs_by_dir, parent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FileMeta;

    #[test]
    fn collect_dedupes_shared_ancestors() {
        // ARRANGE
        let files = &[
            FileMeta::new("EFI/BOOT/BOOTX64.EFI", 11),
            FileMeta::new("EFI/BOOT/second.efi", 5),
            FileMeta::new("cfg.txt", 6),
        ];

        // ACT
        let tree = DirIndex::collect(files);

        // ASSERT
        assert_eq!(tree.len(), 3, "only \"\", \"EFI\" and \"EFI/BOOT\" exist");
        assert_eq!(tree.path(0), "");
        assert_eq!(tree.path(1), "EFI");
        assert_eq!(tree.path(2), "EFI/BOOT");
    }

    #[test]
    fn parent_links_point_at_prefix_directories() {
        // ARRANGE
        let files = &[
            FileMeta::new("EFI/BOOT/BOOTX64.EFI", 11),
            FileMeta::new("cfg.txt", 6),
        ];
        let tree = DirIndex::collect(files);

        // ACT
        let boot_parent = tree.parent_of(2);
        let efi_parent = tree.parent_of(1);
        let root_parent = tree.parent_of(0);

        // ASSERT
        assert_eq!(tree.path(boot_parent), "EFI");
        assert_eq!(tree.path(efi_parent), "");
        assert_eq!(root_parent, 0, "root maps to itself");
    }

    #[test]
    fn children_are_grouped_by_parent_directory() {
        // ARRANGE
        let files = &[
            FileMeta::new("EFI/BOOT/BOOTX64.EFI", 11),
            FileMeta::new("cfg.txt", 6),
            FileMeta::new("EFI/version", 2),
        ];
        let tree = DirIndex::collect(files);

        // ACT
        let boot_dir = tree.subdirs(1).first().copied().unwrap_or(0);
        let boot_files = tree.files(boot_dir);
        let root_files = tree.files(0);

        // ASSERT
        assert_eq!(boot_files, &[0], "BOOTX64.EFI lives in EFI/BOOT");
        assert_eq!(root_files, &[1], "cfg.txt lives in root");
        assert_eq!(tree.files(1), &[2], "version lives in EFI");
    }

    #[test]
    fn sizes_match_the_encoded_entries() {
        // ARRANGE
        let files = &[
            FileMeta::new("EFI/BOOT/BOOTX64.EFI", 11),
            FileMeta::new("cfg.txt", 6),
            FileMeta::new("act-led.dtbo", 2),
        ];
        let tree = DirIndex::collect(files);

        // ACT
        let sizes = tree.sizes(files);
        let size_of = |i: usize| sizes.get(i).copied().unwrap_or(0);

        // ASSERT
        // Root: dot entries + EFI + cfg.txt + act-led.dtbo (LFN: 1 fragment + 8.3).
        assert_eq!(size_of(0), 64 + 32 + 32 + 64, "root size");
        // EFI: dot entries + BOOT.
        assert_eq!(size_of(1), 96, "EFI size");
        // EFI/BOOT: dot entries + BOOTX64.EFI.
        assert_eq!(size_of(2), 96, "EFI/BOOT size");
    }

    #[test]
    fn subdirs_are_emitted_in_sorted_order() {
        // ARRANGE
        let files = &[FileMeta::new("b/one", 1), FileMeta::new("a/two", 1)];
        let tree = DirIndex::collect(files);

        // ACT
        let subdirs = tree.subdirs(0);

        // ASSERT
        assert_eq!(tree.path(subdirs.first().copied().unwrap_or(0)), "a");
        assert_eq!(tree.path(subdirs.get(1).copied().unwrap_or(0)), "b");
    }
}
