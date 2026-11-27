use crate::types::{ClusterMap, FAT32_EOC, FatLayout, ROOT_CLUSTER, SECTOR_SIZE};

pub(crate) fn make_fat(map: &ClusterMap, layout: &FatLayout) -> Vec<u8> {
    let total_entries = usize::try_from(layout.data_cluster_count.wrapping_add(2)).unwrap_or(0);
    let fat_byte_count = layout.fat_sectors.wrapping_mul(SECTOR_SIZE);
    let mut fat: Vec<u32> = vec![0; total_entries];
    if let Some(entry) = fat.get_mut(0) {
        *entry = 0x0FFF_FFF8;
    }
    if let Some(entry) = fat.get_mut(1) {
        *entry = FAT32_EOC;
    }
    for i in 0..map.dir_starts.len() {
        let start = usize::try_from(map.dir_starts.get(i).copied().unwrap_or(0)).unwrap_or(0);
        let count = usize::try_from(map.dir_counts.get(i).copied().unwrap_or(0)).unwrap_or(0);
        fill_fat_chain(&mut fat, start, count, FAT32_EOC);
    }
    for i in 0..map.file_starts.len() {
        let start =
            usize::try_from(map.file_starts.get(i).copied().unwrap_or(ROOT_CLUSTER)).unwrap_or(0);
        let count = usize::try_from(map.file_counts.get(i).copied().unwrap_or(0)).unwrap_or(0);
        fill_fat_chain(&mut fat, start, count, FAT32_EOC);
    }
    let mut bytes = Vec::with_capacity(usize::try_from(fat_byte_count).unwrap_or(0));
    for &e in &fat {
        bytes.extend_from_slice(&e.to_le_bytes());
    }
    let needed = usize::try_from(fat_byte_count).unwrap_or(0);
    if bytes.len() < needed {
        bytes.resize(needed, 0);
    }

    bytes
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
