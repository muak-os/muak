//! FAT directory data construction.

use std::io::Write;

use crate::error::Result;
use crate::name;
use crate::tree::DirIndex;
use crate::types::{ATTR_DIRECTORY, ClusterMap, FatLayout, FileMeta, ROOT_CLUSTER};

pub(crate) fn build_data(
    files: &[FileMeta<'_>],
    tree: &DirIndex<'_>,
    map: &ClusterMap,
    dir_index: usize,
    layout: &FatLayout,
) -> Vec<u8> {
    let cluster_bytes = layout.spc.wrapping_mul(512);
    let me = *map.dir_starts.get(dir_index).unwrap_or(&ROOT_CLUSTER);
    let parent_cluster = *map
        .dir_starts
        .get(tree.parent_of(dir_index))
        .unwrap_or(&ROOT_CLUSTER);
    let mut data = Vec::with_capacity(cluster_bytes.try_into().unwrap_or(0));
    data.extend_from_slice(&dot_entries(me, parent_cluster));
    for sub_index in tree.subdirs(dir_index) {
        let child = *sub_index;
        let cluster = *map.dir_starts.get(child).unwrap_or(&ROOT_CLUSTER);
        let idx = data.len().next_multiple_of(32);
        data.resize(idx, 0);
        name::append_entry(
            &mut data,
            DirIndex::name_of(tree.path(child)),
            true,
            cluster,
            0,
        );
    }
    for file_index in tree.files(dir_index) {
        let Some(file) = files.get(*file_index) else {
            continue;
        };
        let cluster = map
            .file_starts
            .get(*file_index)
            .copied()
            .unwrap_or(ROOT_CLUSTER);
        let size = u32::try_from(file.size).unwrap_or(u32::MAX);
        let idx = data.len().next_multiple_of(32);
        data.resize(idx, 0);
        name::append_entry(
            &mut data,
            DirIndex::name_of(file.path),
            false,
            cluster,
            size,
        );
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

fn dot_entries(me_cluster: u32, parent_cluster: u32) -> [u8; 64] {
    let mut entries = [0_u8; 64];
    if let Some(slot) = entries.get_mut(..32) {
        slot.copy_from_slice(&name::short_entry(
            &dot_entry(b"."),
            ATTR_DIRECTORY,
            me_cluster,
            0,
        ));
    }
    if let Some(slot) = entries.get_mut(32..64) {
        slot.copy_from_slice(&name::short_entry(
            &dot_entry(b".."),
            ATTR_DIRECTORY,
            parent_cluster,
            0,
        ));
    }

    entries
}

fn dot_entry(name: &[u8]) -> [u8; 11] {
    let mut sn = [b' '; 11];
    for (i, &byte) in name.iter().enumerate().take(11) {
        if let Some(slot) = sn.get_mut(i) {
            *slot = byte;
        }
    }

    sn
}
