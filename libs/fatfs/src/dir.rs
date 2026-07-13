use std::io::Write;

use crate::error::Result;
use crate::types::{
    ATTR_ARCHIVE, ATTR_DIRECTORY, ATTR_LFN, ClusterMap, FatKind, FatLayout, FileMeta, ROOT_CLUSTER,
    fat12_16_cluster, fat32_cluster,
};

pub(crate) fn build_data(
    files: &[FileMeta<'_>],
    dirs: &[String],
    map: &ClusterMap,
    dir_index: usize,
    layout: &FatLayout,
) -> Vec<u8> {
    let cluster_bytes = layout.spc.wrapping_mul(512);
    let me = dir_cluster(dir_index, layout);
    let parent_idx = parent_dir_index(dirs, dir_index);
    let parent_cluster = dir_cluster(parent_idx, layout);
    let capacity = usize::try_from(cluster_bytes).unwrap_or(0);
    let mut data = Vec::with_capacity(capacity);
    data.extend_from_slice(&short_entry(&dot_entry(b"."), ATTR_DIRECTORY, me, 0));
    data.extend_from_slice(&short_entry(
        &dot_entry(b".."),
        ATTR_DIRECTORY,
        parent_cluster,
        0,
    ));
    for (other_idx, dir_path) in dirs.iter().enumerate() {
        if other_idx == dir_index {
            continue;
        }
        let other_parent = std::path::Path::new(dir_path)
            .parent()
            .unwrap_or(std::path::Path::new(""))
            .to_string_lossy()
            .into_owned();
        if dirs.get(dir_index).is_none_or(|dir| other_parent != *dir) {
            continue;
        }
        let name = std::path::Path::new(dir_path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let idx = data.len().next_multiple_of(32);
        data.resize(idx, 0);
        data.extend_from_slice(&dir_entry_bytes(&name, dir_cluster(other_idx, layout)));
    }
    for (file_index, file) in files.iter().enumerate() {
        let fp = std::path::Path::new(file.path);
        let parent = fp.parent().unwrap_or(std::path::Path::new(""));
        let parent_str = parent.to_string_lossy().into_owned();
        if dirs.get(dir_index).is_none_or(|dir| parent_str != *dir) {
            continue;
        }
        let file_name = fp
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let cluster = map
            .file_starts
            .get(file_index)
            .copied()
            .unwrap_or(ROOT_CLUSTER);
        let size = u32::try_from(file.size).unwrap_or(u32::MAX);
        let idx = data.len().next_multiple_of(32);
        data.resize(idx, 0);
        data.extend_from_slice(&file_entry_bytes(&file_name, cluster, size));
    }

    data
}

fn parent_dir_index(dirs: &[String], dir_index: usize) -> usize {
    if dirs.get(dir_index).is_none_or(String::is_empty) {
        return 0;
    }
    let parent_dir = dirs
        .get(dir_index)
        .and_then(|dir| std::path::Path::new(dir).parent())
        .map(|parent| parent.to_string_lossy().into_owned())
        .unwrap_or_default();

    dirs.iter().position(|dir| *dir == parent_dir).unwrap_or(0)
}

fn dir_cluster(index: usize, layout: &FatLayout) -> u32 {
    if layout.kind == FatKind::Fat32 {
        fat32_cluster(index)
    } else {
        fat12_16_cluster(index)
    }
}

fn short_entry(name: &[u8; 11], attr: u8, cluster: u32, size: u32) -> [u8; 32] {
    let mut e = [0_u8; 32];
    if let Some(slot) = e.get_mut(..11) {
        slot.copy_from_slice(name);
    }
    if let Some(slot) = e.get_mut(11) {
        *slot = attr;
    }
    let hi = u16::try_from(cluster.wrapping_shr(16)).unwrap_or(0);
    let lo = u16::try_from(cluster).unwrap_or(0);
    if let Some(slot) = e.get_mut(20..22) {
        slot.copy_from_slice(&hi.to_le_bytes());
    }
    if let Some(slot) = e.get_mut(26..28) {
        slot.copy_from_slice(&lo.to_le_bytes());
    }
    if let Some(slot) = e.get_mut(28..32) {
        slot.copy_from_slice(&size.to_le_bytes());
    }

    e
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

fn file_entry_bytes(name: &str, cluster: u32, size: u32) -> Vec<u8> {
    let short = make_short_name(name, 0);
    if short_fits(&short, name) {
        return short_entry(&short, ATTR_ARCHIVE, cluster, size).to_vec();
    }
    let short = make_short_name(name, 1);
    let csum = lfn_checksum(&short);
    let lfns = lfn_entries(name, csum);
    let mut bytes = Vec::with_capacity(lfns.len().wrapping_add(1).wrapping_mul(32));
    for lfn_entry in &lfns {
        bytes.extend_from_slice(lfn_entry);
    }
    bytes.extend_from_slice(&short_entry(&short, ATTR_ARCHIVE, cluster, size));

    bytes
}

fn dir_entry_bytes(name: &str, cluster: u32) -> Vec<u8> {
    let short = make_short_name(name, 0);
    if short_fits(&short, name) {
        return short_entry(&short, ATTR_DIRECTORY, cluster, 0).to_vec();
    }
    let short = make_short_name(name, 1);
    let csum = lfn_checksum(&short);
    let lfns = lfn_entries(name, csum);
    let mut bytes = Vec::with_capacity(lfns.len().wrapping_add(1).wrapping_mul(32));
    for lfn_entry in &lfns {
        bytes.extend_from_slice(lfn_entry);
    }
    bytes.extend_from_slice(&short_entry(&short, ATTR_DIRECTORY, cluster, 0));

    bytes
}

fn make_short_name(name: &str, seq: u8) -> [u8; 11] {
    let mut sn = [b' '; 11];
    let upper: Vec<u8> = name.to_uppercase().into_bytes();
    let dot_index = upper.iter().rposition(|&byte| byte == b'.');
    let (root, extension) = if let Some(dot_pos) = dot_index {
        (
            upper.get(..dot_pos).unwrap_or(&[]),
            upper.get(dot_pos.wrapping_add(1)..).unwrap_or(&[]),
        )
    } else {
        (&*upper, &[][..])
    };
    let tilde = seq > 0;
    let base_max: usize = if tilde { 6 } else { 8 };
    for (i, &byte) in root.iter().enumerate().take(base_max) {
        if let Some(slot) = sn.get_mut(i) {
            *slot = valid_char(byte);
        }
    }
    if tilde {
        if let Some(slot) = sn.get_mut(6) {
            *slot = b'~';
        }
        if let Some(slot) = sn.get_mut(7) {
            *slot = b'0'.wrapping_add(seq.min(9));
        }
    }
    for (i, &byte) in extension.iter().enumerate().take(3) {
        if let Some(slot) = sn.get_mut(8_usize.wrapping_add(i)) {
            *slot = valid_char(byte);
        }
    }

    sn
}

fn short_fits(short: &[u8; 11], name: &str) -> bool {
    let upper: Vec<u8> = name.to_uppercase().into_bytes();
    let dot_index = upper.iter().rposition(|&byte| byte == b'.');
    let (root, extension) = if let Some(dot_pos) = dot_index {
        (
            upper.get(..dot_pos).unwrap_or(&[]),
            upper.get(dot_pos.wrapping_add(1)..).unwrap_or(&[]),
        )
    } else {
        (&*upper, &[][..])
    };
    if root.len() > 8 || extension.len() > 3 {
        return false;
    }
    for (i, &byte) in root.iter().enumerate() {
        match short.get(i) {
            Some(&short_byte) if short_byte != valid_char(byte) => return false,
            None => return false,
            _ => {}
        }
    }
    for (i, &byte) in extension.iter().enumerate() {
        match short.get(8_usize.wrapping_add(i)) {
            Some(&short_byte) if short_byte != valid_char(byte) => return false,
            None => return false,
            _ => {}
        }
    }

    true
}

fn valid_char(byte: u8) -> u8 {
    if byte.is_ascii_alphanumeric() || b"_^$~!#%&-@'(){}".contains(&byte) {
        byte
    } else {
        b'_'
    }
}

fn lfn_checksum(short: &[u8; 11]) -> u8 {
    let mut sum = 0_u8;
    for &byte in short {
        sum = sum.wrapping_add(byte).rotate_right(1);
    }

    sum
}

fn lfn_entries(name: &str, checksum: u8) -> Vec<[u8; 32]> {
    let utf16: Vec<u16> = name.encode_utf16().collect();
    let chunks: Vec<&[u16]> = utf16.chunks(13).collect();
    let total = chunks.len();
    let mut entries = Vec::with_capacity(chunks.len());
    for (i, chunk) in chunks.iter().enumerate().rev() {
        let mut ent = [0_u8; 32];
        let ordinal = if i == 0 { 0x40 } else { 0 };
        let ordinal_val = u8::try_from(total.wrapping_sub(i)).unwrap_or(0);
        if let Some(slot) = ent.get_mut(0) {
            *slot = ordinal | ordinal_val;
        }
        if let Some(slot) = ent.get_mut(11) {
            *slot = ATTR_LFN;
        }
        if let Some(slot) = ent.get_mut(13) {
            *slot = checksum;
        }
        write_lfn_chars(&mut ent, chunk);
        entries.push(ent);
    }

    entries
}

fn write_lfn_chars(ent: &mut [u8; 32], chars: &[u16]) {
    for (j, &cp) in chars.iter().enumerate() {
        let off = lfn_offset(j);
        let end = off.wrapping_add(2);
        if let Some(slot) = ent.get_mut(off..end) {
            slot.copy_from_slice(&cp.to_le_bytes());
        }
    }
}

fn lfn_offset(index: usize) -> usize {
    if index < 5 {
        1_usize.wrapping_add(index.wrapping_mul(2))
    } else if index < 11 {
        14_usize.wrapping_add(index.wrapping_sub(5).wrapping_mul(2))
    } else {
        28_usize.wrapping_add(index.wrapping_sub(11).wrapping_mul(2))
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_name_truncation() {
        // ARRANGE / ACT
        let sn = make_short_name("VeryLong.Extra", 1);

        // ASSERT
        assert_eq!(sn.get(..6), Some(&b"VERYLO"[..]));
        assert_eq!(sn.get(6), Some(&b'~'));
        assert_eq!(sn.get(7), Some(&b'1'));
        assert_eq!(sn.get(8..11), Some(&b"EXT"[..]));
    }

    #[test]
    fn short_name_no_tilde() {
        // ARRANGE / ACT
        let sn = make_short_name("BOOTX64.EFI", 0);

        // ASSERT
        assert_eq!(sn.get(..8), Some(&b"BOOTX64 "[..]));
        assert_eq!(sn.get(8..11), Some(&b"EFI"[..]));
    }

    #[test]
    fn lfn_checksum_test() {
        // ARRANGE
        let sn = make_short_name("BOOTX64.EFI", 0);

        // ACT
        let checksum = lfn_checksum(&sn);

        // ASSERT
        assert!(checksum > 0);
    }
}
