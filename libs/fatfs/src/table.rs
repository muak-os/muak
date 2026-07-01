use crate::types::{
    ClusterMap, FAT12_EOC, FAT16_EOC, FAT32_EOC, FatKind, FatLayout, ROOT_CLUSTER, SECTOR_SIZE,
};

pub(crate) fn make_fat(map: &ClusterMap, layout: &FatLayout) -> Vec<u8> {
    let total_entries = usize::try_from(layout.data_cluster_count.wrapping_add(2)).unwrap_or(0);
    let fat_byte_count = layout.fat_sectors.wrapping_mul(SECTOR_SIZE);
    let mut fat: Vec<u32> = vec![0; total_entries];
    let eoc = eoc_value(layout.kind);
    if let Some(entry) = fat.get_mut(0) {
        *entry = match layout.kind {
            FatKind::Fat12 => 0x0FF8,
            FatKind::Fat16 => 0xFFF8,
            FatKind::Fat32 => 0x0FFF_FFF8,
        };
    }
    if let Some(entry) = fat.get_mut(1) {
        *entry = eoc;
    }
    for &dc in &map.dir_clusters {
        if dc < 2 {
            continue;
        }
        let dc_idx = usize::try_from(dc).unwrap_or(0);
        if let Some(entry) = fat.get_mut(dc_idx) {
            *entry = eoc;
        }
    }
    for i in 0..map.file_starts.len() {
        let start =
            usize::try_from(map.file_starts.get(i).copied().unwrap_or(ROOT_CLUSTER)).unwrap_or(0);
        let count = usize::try_from(map.file_counts.get(i).copied().unwrap_or(0)).unwrap_or(0);
        fill_fat_chain(&mut fat, start, count, eoc);
    }
    let mut bytes = Vec::with_capacity(usize::try_from(fat_byte_count).unwrap_or(0));
    match layout.kind {
        FatKind::Fat12 => {
            pack_fat12_entries(&fat, &mut bytes);
        }
        FatKind::Fat16 => {
            for &e in &fat {
                bytes.extend_from_slice(&u16::try_from(e).unwrap_or(0).to_le_bytes());
            }
        }
        FatKind::Fat32 => {
            for &e in &fat {
                bytes.extend_from_slice(&e.to_le_bytes());
            }
        }
    }
    let needed = usize::try_from(fat_byte_count).unwrap_or(0);
    if bytes.len() < needed {
        bytes.resize(needed, 0);
    }

    bytes
}

fn pack_fat12_entries(fat: &[u32], bytes: &mut Vec<u8>) {
    for i in (0..fat.len()).step_by(2) {
        let e = fat.get(i).copied().unwrap_or(0);
        let next = fat.get(i.wrapping_add(1)).copied().unwrap_or(0);
        let packed = (e & 0x0FFF) | ((next & 0x0FFF) << 12);
        bytes.extend_from_slice(packed.to_le_bytes().get(..3).unwrap_or(&[0; 3]));
    }
}

fn fill_fat_chain(fat: &mut [u32], start: usize, count: usize, eoc: u32) {
    for j in 0..count {
        let value = if j.wrapping_add(1) < count {
            u32::try_from(start.wrapping_add(j).wrapping_add(1)).unwrap_or(eoc)
        } else {
            eoc
        };
        let idx = start.wrapping_add(j);
        if let Some(entry) = fat.get_mut(idx) {
            *entry = value;
        }
    }
}

fn eoc_value(kind: FatKind) -> u32 {
    match kind {
        FatKind::Fat12 => FAT12_EOC,
        FatKind::Fat16 => FAT16_EOC,
        FatKind::Fat32 => FAT32_EOC,
    }
}
