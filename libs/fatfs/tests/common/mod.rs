#![cfg(test)]

//! Independent FAT32 reader and conformance assertions shared by the fatfs tests.

use std::collections::HashSet;

use fatfs::builder::{self, image_size_for};
use fatfs::types::FileMeta;

/// Bytes in one FAT sector.
pub const SECTOR_SIZE: usize = 512;
/// Bytes in one directory entry.
pub const ENTRY_SIZE: usize = 32;
/// Attribute value of a long file name entry.
pub const ATTR_LFN: u8 = 0x0F;
/// Attribute bit of a directory.
pub const ATTR_DIRECTORY: u8 = 0x10;
/// Attribute value of the volume label entry.
pub const ATTR_VOLUME_ID: u8 = 0x08;
/// Deepest directory level the reader walks.
const MAX_DEPTH: usize = 4;
/// End-of-chain threshold of the FAT.
const EOC: u32 = 0x0FFF_FFF8;

/// One directory entry as found on disk.
#[derive(Clone, Debug)]
pub struct Entry {
    /// Path relative to the volume root.
    pub path: String,
    /// Name decoded from the LFN entries, or the 8.3 name when they are absent.
    pub long_name: String,
    /// Raw 8.3 name field.
    pub short_name: [u8; 11],
    /// Attribute byte.
    pub attr: u8,
    /// First cluster.
    pub cluster: u32,
    /// Declared size in bytes.
    pub size: u32,
    /// Creation, access and write dates.
    pub dates: [u16; 3],
    /// Whether the LFN entries of this name are well formed.
    pub lfn_ok: bool,
}

impl Entry {
    /// Returns whether the entry is a file.
    pub fn is_file(&self) -> bool {
        self.attr & ATTR_DIRECTORY == 0
    }
}

/// A parsed FAT32 volume.
pub struct Volume<'a> {
    bytes: &'a [u8],
}

impl<'a> Volume<'a> {
    /// Wraps a raw volume image.
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    /// Length of the volume image in bytes.
    pub fn image_len(&self) -> usize {
        self.bytes.len()
    }

    /// Bytes per logical sector from the BPB.
    pub fn bytes_per_sector(&self) -> usize {
        usize::from(word(self.bytes, 11))
    }

    /// Sectors per cluster from the BPB.
    pub fn sectors_per_cluster(&self) -> usize {
        usize::from(byte(self.bytes, 13))
    }

    /// Reserved sector count from the BPB.
    pub fn reserved_sectors(&self) -> usize {
        usize::from(word(self.bytes, 14))
    }

    /// Number of FAT copies from the BPB.
    pub fn fat_count(&self) -> usize {
        usize::from(byte(self.bytes, 16))
    }

    /// Sectors per FAT from the BPB.
    pub fn fat_sectors(&self) -> usize {
        usize::try_from(dword(self.bytes, 36)).unwrap_or(0)
    }

    /// Total sectors from the BPB.
    pub fn total_sectors(&self) -> u32 {
        dword(self.bytes, 32)
    }

    /// Root directory cluster from the BPB.
    pub fn root_cluster(&self) -> u32 {
        dword(self.bytes, 44)
    }

    /// `FSInfo` sector from the BPB.
    pub fn fsinfo_sector(&self) -> usize {
        usize::from(word(self.bytes, 48))
    }

    /// Backup boot sector from the BPB.
    pub fn backup_boot_sector(&self) -> usize {
        usize::from(word(self.bytes, 50))
    }

    /// Volume label from the BPB.
    pub fn label(&self) -> [u8; 11] {
        array(self.bytes.get(71..82).unwrap_or(&[]))
    }

    /// Bytes per cluster.
    pub fn cluster_bytes(&self) -> usize {
        self.bytes_per_sector()
            .saturating_mul(self.sectors_per_cluster())
    }

    /// Byte offset of the data region.
    pub fn data_offset(&self) -> usize {
        let fats = self.fat_count().saturating_mul(self.fat_sectors());

        self.reserved_sectors()
            .saturating_add(fats)
            .saturating_mul(self.bytes_per_sector())
    }

    /// Number of data clusters.
    pub fn cluster_count(&self) -> usize {
        let fats = self.fat_count().saturating_mul(self.fat_sectors());
        let overhead = self.reserved_sectors().saturating_add(fats);
        let sectors = usize::try_from(self.total_sectors())
            .unwrap_or(0)
            .saturating_sub(overhead);

        sectors.div_euclid(self.sectors_per_cluster())
    }

    /// Free cluster count from the `FSInfo` sector.
    pub fn free_clusters(&self) -> u32 {
        let offset = self
            .fsinfo_sector()
            .saturating_mul(self.bytes_per_sector())
            .saturating_add(488);

        dword(self.bytes, offset)
    }

    /// One sector of the image.
    pub fn sector(&self, index: usize) -> &'a [u8] {
        let start = index.saturating_mul(SECTOR_SIZE);

        self.bytes
            .get(start..start.saturating_add(SECTOR_SIZE))
            .unwrap_or(&[])
    }

    /// Data bytes of one cluster.
    pub fn cluster_data(&self, cluster: u32) -> &'a [u8] {
        let index = usize::try_from(cluster.saturating_sub(2)).unwrap_or(usize::MAX);
        let start = self
            .data_offset()
            .saturating_add(self.cluster_bytes().saturating_mul(index));

        self.bytes
            .get(start..start.saturating_add(self.cluster_bytes()))
            .unwrap_or(&[])
    }

    /// The first FAT copy, limited to the valid data clusters.
    pub fn fat(&self) -> Vec<u32> {
        let start = self
            .reserved_sectors()
            .saturating_mul(self.bytes_per_sector());

        (0..=self.cluster_count().saturating_add(1))
            .map(|index| {
                dword(self.bytes, start.saturating_add(index.saturating_mul(4))) & 0x0FFF_FFFF
            })
            .collect()
    }

    /// Follows a cluster chain until its end marker.
    pub fn chain(&self, start: u32) -> Vec<u32> {
        let fat = self.fat();
        let mut chain = Vec::new();
        let mut current = Some(start);
        while let Some(cluster) = current.filter(|&cluster| cluster >= 2) {
            chain.push(cluster);
            current = next_in_chain(&fat, cluster);
        }

        chain
    }

    /// Concatenated bytes of a cluster chain.
    pub fn chain_data(&self, start: u32) -> Vec<u8> {
        self.chain(start)
            .into_iter()
            .flat_map(|cluster| self.cluster_data(cluster).to_vec())
            .collect()
    }

    /// Volume label entries found in the root directory.
    pub fn label_entries(&self) -> Vec<[u8; 11]> {
        raw_dir(self, self.root_cluster())
            .into_iter()
            .filter(|entry| byte(entry, 11) == ATTR_VOLUME_ID)
            .map(|entry| array(&entry))
            .collect()
    }

    /// Every entry of the volume, excluding `.`, `..` and volume labels.
    pub fn entries(&self) -> Vec<Entry> {
        let mut entries = Vec::new();
        walk(self, self.root_cluster(), "", &mut entries, 0);

        entries
    }
}

/// Builds a volume from `(path, payload)` pairs, sized by the public API.
pub fn build_image(files: &[(&str, &[u8])]) -> Vec<u8> {
    let total = files
        .iter()
        .map(|&(_, payload)| u64::try_from(payload.len()).unwrap_or(0))
        .sum::<u64>();
    let image_size = image_size_for(total).expect("image size must compute");

    build_image_sized(files, image_size)
}

/// Builds a volume of an explicit size from `(path, payload)` pairs.
pub fn build_image_sized(files: &[(&str, &[u8])], image_size: u64) -> Vec<u8> {
    let metas: Vec<FileMeta<'_>> = files
        .iter()
        .map(|&(path, payload)| {
            FileMeta::new(path, u64::try_from(payload.len()).unwrap_or(u64::MAX))
        })
        .collect();
    let precomputed = builder::precompute(&metas, image_size).expect("precompute must succeed");
    let mut readers: Vec<&[u8]> = files.iter().map(|&(_, payload)| payload).collect();
    let mut out = Vec::new();
    builder::build(&precomputed, &mut readers, &mut out).expect("build must succeed");

    out
}

/// Asserts that the boot sector describes a FAT32 volume.
pub fn assert_boot_sector(volume: &Volume<'_>) {
    assert_eq!(volume.bytes_per_sector(), SECTOR_SIZE, "bytes per sector");
    assert_eq!(volume.fat_count(), 2, "FAT copies");
    assert_eq!(volume.root_cluster(), 2, "root cluster");
    assert_eq!(volume.fsinfo_sector(), 1, "FSInfo sector");
    assert_eq!(volume.backup_boot_sector(), 6, "backup boot sector");
    assert_eq!(volume.sector(0).get(510..512), Some(&[0x55, 0xAA][..]));
    assert_eq!(volume.sector(0).get(82..90), Some(&b"FAT32   "[..]));
    assert_eq!(
        usize::try_from(volume.total_sectors())
            .unwrap_or(0)
            .saturating_mul(SECTOR_SIZE),
        volume.image_len(),
        "total sectors must cover the image"
    );
    assert!(
        volume.cluster_count() >= 65_525,
        "FAT32 needs at least 65525 clusters, got {}",
        volume.cluster_count()
    );
}

/// Asserts that the backup boot sector copies the primary one.
pub fn assert_backup_boot_sector(volume: &Volume<'_>) {
    assert_eq!(
        volume.sector(0),
        volume.sector(volume.backup_boot_sector()),
        "the backup boot sector must copy sector 0"
    );
}

/// Asserts that the `FSInfo` free count matches the FAT.
pub fn assert_fsinfo(volume: &Volume<'_>) {
    let free = volume
        .fat()
        .iter()
        .skip(2)
        .filter(|&&entry| entry == 0)
        .count();
    assert_eq!(
        usize::try_from(volume.free_clusters()).unwrap_or(usize::MAX),
        free,
        "FSInfo free cluster count"
    );
}

/// Asserts that the volume label is consistent and distinct from directory names.
pub fn assert_label(volume: &Volume<'_>, entries: &[Entry]) {
    let label = volume.label();
    let labels = volume.label_entries();
    assert_eq!(labels.len(), 1, "exactly one label entry");
    assert_eq!(labels.first(), Some(&label), "BPB and root label agree");
    let collisions = entries
        .iter()
        .filter(|entry| !entry.is_file())
        .filter(|entry| entry.short_name == label)
        .count();
    assert_eq!(
        collisions, 0,
        "the label must not collide with a directory name"
    );
}

/// Asserts that every 8.3 alias of a directory is unique.
pub fn assert_unique_short_names(entries: &[Entry]) {
    let mut seen: HashSet<(String, [u8; 11])> = HashSet::new();
    for entry in entries {
        let parent = entry
            .path
            .rsplit_once('/')
            .map_or(String::new(), |(dir, _)| dir.to_owned());
        assert!(
            seen.insert((parent.clone(), entry.short_name)),
            "duplicate 8.3 alias {} in {parent:?} for {}",
            short_alias(&entry.short_name),
            entry.path
        );
    }
}

/// Asserts that looking a file up by its 8.3 alias finds that very file.
pub fn assert_short_name_lookup(entries: &[Entry]) {
    for entry in entries {
        let found = entries.iter().find(|other| {
            other.short_name == entry.short_name && same_parent(&other.path, &entry.path)
        });
        assert_eq!(
            found.map(|found| found.path.as_str()),
            Some(entry.path.as_str()),
            "8.3 lookup of {}",
            entry.path
        );
    }
}

/// Asserts that every LFN entry belongs to its 8.3 entry.
pub fn assert_lfn_checksums(entries: &[Entry]) {
    for entry in entries {
        assert!(
            entry.lfn_ok,
            "the LFN entries of {} must match its 8.3 checksum",
            entry.path
        );
    }
}

/// Asserts that every timestamp is a valid FAT date.
pub fn assert_timestamps(entries: &[Entry]) {
    for entry in entries {
        for date in entry.dates {
            let month = date.wrapping_shr(5) & 0xF;
            let day = date & 0x1F;
            assert!(
                (1..=12).contains(&month) && (1..=31).contains(&day),
                "invalid FAT date {date:#06x} on {}",
                entry.path
            );
        }
    }
}

/// Asserts that chains match the declared sizes and never overlap.
pub fn assert_chains(volume: &Volume<'_>, entries: &[Entry]) {
    let mut used: HashSet<u32> = volume.chain(volume.root_cluster()).into_iter().collect();
    for entry in entries {
        let chain = volume.chain(entry.cluster);
        if entry.is_file() {
            let expected = usize::try_from(entry.size)
                .unwrap_or(0)
                .div_ceil(volume.cluster_bytes());
            assert_eq!(
                chain.len(),
                expected,
                "chain length of {} ({} bytes)",
                entry.path,
                entry.size
            );
        }
        for cluster in chain {
            assert!(
                used.insert(cluster),
                "cluster {cluster} is shared by {}",
                entry.path
            );
        }
    }
}

/// Asserts that the allocated clusters are exactly the reachable ones.
pub fn assert_no_lost_clusters(volume: &Volume<'_>, entries: &[Entry]) {
    let mut reachable: HashSet<u32> = volume.chain(volume.root_cluster()).into_iter().collect();
    for entry in entries {
        reachable.extend(volume.chain(entry.cluster));
    }
    let allocated = volume
        .fat()
        .iter()
        .skip(2)
        .filter(|&&entry| entry != 0)
        .count();
    assert_eq!(
        reachable.len(),
        allocated,
        "every allocated cluster must be reachable"
    );
}

/// Asserts that the file contents were streamed at their declared offsets.
pub fn assert_contents(volume: &Volume<'_>, expected: &[(&str, &[u8])]) {
    let entries = volume.entries();
    for &(path, payload) in expected {
        let found = entries
            .iter()
            .find(|entry| entry.path.eq_ignore_ascii_case(path));
        assert!(found.is_some(), "missing entry {path}");
        let data = found.map_or_else(Vec::new, |entry| volume.chain_data(entry.cluster));
        assert_eq!(
            data.get(..payload.len()),
            Some(payload),
            "content of {path}"
        );
    }
}

/// Runs every structural assertion on a volume.
pub fn assert_conformant(volume: &Volume<'_>) {
    let entries = volume.entries();
    assert_boot_sector(volume);
    assert_backup_boot_sector(volume);
    assert_fsinfo(volume);
    assert_label(volume, &entries);
    assert_unique_short_names(&entries);
    assert_short_name_lookup(&entries);
    assert_lfn_checksums(&entries);
    assert_timestamps(&entries);
    assert_chains(volume, &entries);
    assert_no_lost_clusters(volume, &entries);
}

/// Renders the 8.3 name field as `BASE.EXT`.
pub fn short_alias(name: &[u8; 11]) -> String {
    let base = String::from_utf8_lossy(name.get(..8).unwrap_or(&[]))
        .trim_end()
        .to_owned();
    let ext = String::from_utf8_lossy(name.get(8..).unwrap_or(&[]))
        .trim_end()
        .to_owned();
    if ext.is_empty() {
        return base;
    }

    format!("{base}.{ext}")
}

fn walk(volume: &Volume<'_>, start: u32, prefix: &str, out: &mut Vec<Entry>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    let parsed = parse_dir(volume, start, prefix);
    let subdirs: Vec<(u32, String)> = parsed
        .iter()
        .filter(|entry| !entry.is_file())
        .map(|entry| (entry.cluster, entry.path.clone()))
        .collect();
    out.extend(parsed);
    for (cluster, path) in subdirs {
        walk(
            volume,
            cluster,
            &format!("{path}/"),
            out,
            depth.saturating_add(1),
        );
    }
}

fn parse_dir(volume: &Volume<'_>, start: u32, prefix: &str) -> Vec<Entry> {
    let mut pending: Vec<[u8; ENTRY_SIZE]> = Vec::new();
    let mut out = Vec::new();
    for raw in raw_dir(volume, start)
        .into_iter()
        .take_while(|raw| byte(raw, 0) != 0)
    {
        if byte(&raw, 11) == ATTR_LFN {
            pending.push(raw);
            continue;
        }
        if let Some(entry) = parse_entry(&raw, &pending, prefix) {
            out.push(entry);
        }
        pending.clear();
    }

    out
}

fn parse_entry(
    raw: &[u8; ENTRY_SIZE],
    pending: &[[u8; ENTRY_SIZE]],
    prefix: &str,
) -> Option<Entry> {
    let short_name = array(raw);
    let lfn_ok = lfn_is_valid(pending, &short_name);
    let long_name = if lfn_ok && !pending.is_empty() {
        decode_lfn(pending)
    } else {
        short_alias(&short_name)
    };
    let attr = byte(raw, 11);
    let is_visible = long_name != "." && long_name != ".." && attr != ATTR_VOLUME_ID;
    let path = format!("{prefix}{long_name}");
    let entry = Entry {
        path,
        long_name,
        short_name,
        attr,
        cluster: (u32::from(word(raw, 20)).wrapping_shl(16)) | u32::from(word(raw, 26)),
        size: dword(raw, 28),
        dates: [word(raw, 16), word(raw, 18), word(raw, 24)],
        lfn_ok,
    };

    is_visible.then_some(entry)
}

fn raw_dir(volume: &Volume<'_>, start: u32) -> Vec<[u8; ENTRY_SIZE]> {
    volume
        .chain_data(start)
        .chunks(ENTRY_SIZE)
        .filter(|chunk| chunk.len() == ENTRY_SIZE)
        .map(array)
        .collect()
}

fn next_in_chain(fat: &[u32], cluster: u32) -> Option<u32> {
    let next = *fat.get(usize::try_from(cluster).ok()?)?;
    if next >= EOC {
        return None;
    }

    Some(next)
}

fn same_parent(left: &str, right: &str) -> bool {
    left.rsplit_once('/').map(|(dir, _)| dir) == right.rsplit_once('/').map(|(dir, _)| dir)
}

fn lfn_is_valid(entries: &[[u8; ENTRY_SIZE]], short_name: &[u8; 11]) -> bool {
    if entries.is_empty() {
        return true;
    }
    let checksum = lfn_checksum(short_name);
    let ordinals: Vec<u8> = entries.iter().map(|entry| byte(entry, 0)).collect();
    let total = ordinals.first().map_or(0, |ordinal| ordinal & 0x3F);
    let flags_ok = ordinals.first().is_some_and(|ordinal| ordinal & 0x40 != 0);
    let checksums_ok = entries.iter().all(|entry| byte(entry, 13) == checksum);
    let order_ok = ordinals.iter().enumerate().all(|(index, ordinal)| {
        ordinal & 0x3F == total.saturating_sub(u8::try_from(index).unwrap_or(u8::MAX))
    });

    flags_ok && checksums_ok && order_ok && usize::from(total) == ordinals.len()
}

fn lfn_checksum(name: &[u8; 11]) -> u8 {
    let mut sum = 0_u8;
    for &byte in name {
        sum = sum.rotate_right(1).wrapping_add(byte);
    }

    sum
}

fn decode_lfn(entries: &[[u8; ENTRY_SIZE]]) -> String {
    let mut name = String::new();
    for code in entries.iter().rev().flat_map(fragment_codes) {
        if code == 0 || code == 0xFFFF {
            break;
        }
        name.push(char::from_u32(u32::from(code)).unwrap_or('\u{FFFD}'));
    }

    name
}

fn fragment_codes(entry: &[u8; ENTRY_SIZE]) -> impl Iterator<Item = u16> + '_ {
    const OFFSETS: [usize; 13] = [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];

    OFFSETS.into_iter().map(move |offset| word(entry, offset))
}

fn byte(buf: &[u8], offset: usize) -> u8 {
    buf.get(offset).copied().unwrap_or(0)
}

fn word(buf: &[u8], offset: usize) -> u16 {
    buf.get(offset..offset.saturating_add(2))
        .and_then(|bytes| bytes.try_into().ok())
        .map_or(0, u16::from_le_bytes)
}

fn dword(buf: &[u8], offset: usize) -> u32 {
    buf.get(offset..offset.saturating_add(4))
        .and_then(|bytes| bytes.try_into().ok())
        .map_or(0, u32::from_le_bytes)
}

fn array<const N: usize>(src: &[u8]) -> [u8; N] {
    let mut out = [0_u8; N];
    for (slot, &byte) in out.iter_mut().zip(src) {
        *slot = byte;
    }

    out
}
