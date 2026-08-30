//! FAT image building API.

use std::io::{Read, Write};

use crate::boot;
use crate::dir;
use crate::error::{FatError, Result};
use crate::table;
use crate::types::{
    ClusterMap, FAT_COUNT, FAT_ENTRY_SIZE, FAT32_MIN_CLUSTERS, FatLayout, FileMeta, MAX_IMAGE_SIZE,
    Precomputed, RESERVED_SECTORS, ROOT_CLUSTER, SECTOR_SIZE,
};

/// Precomputes all FAT metadata from file paths and sizes.
///
/// # Errors
///
/// Returns `Error::Fat` when layout computation fails or files don't fit.
pub fn precompute(files: &[FileMeta<'_>], image_size: u64) -> Result<Precomputed> {
    let layout = compute_layout(image_size)?;
    let dirs = collect_dir_paths(files);
    let probe = ClusterMap {
        dir_starts: vec![0; dirs.len()],
        dir_counts: vec![0; dirs.len()],
        file_starts: vec![0; files.len()],
        file_counts: vec![0; files.len()],
        file_sizes: files.iter().map(|file| file.size).collect(),
    };
    let probe_dirs = build_all_dir_data(files, &dirs, &probe, &layout);
    let dir_sizes: Vec<u64> = probe_dirs
        .iter()
        .map(|data| u64::try_from(data.len()).unwrap_or(u64::MAX))
        .collect();
    let cluster_map = assign_clusters(files, &dirs, &layout, &dir_sizes)?;
    let fat_bytes = table::make_fat(&cluster_map, &layout);
    let dir_data: Vec<Vec<u8>> = build_all_dir_data(files, &dirs, &cluster_map, &layout);

    Ok(Precomputed {
        layout,
        dirs,
        cluster_map,
        fat_bytes,
        dir_data,
        image_size,
    })
}

/// Builds a FAT image by writing precomputed metadata and streaming file data.
///
/// # Errors
///
/// Returns `Error::Io` when writing fails, or `Error::Fat` when a reader
/// returns EOF before its declared size.
pub fn build<R: Read, W: Write>(
    precomputed: &Precomputed,
    readers: &mut [R],
    writer: &mut W,
) -> Result<()> {
    let mut cw = CountWriter {
        inner: writer,
        written: 0,
    };
    let cluster_bytes = precomputed.layout.spc.wrapping_mul(SECTOR_SIZE);

    write_boot_sector(&mut cw, &precomputed.layout)?;
    write_reserved_padding(&mut cw, &precomputed.layout)?;

    cw.write_all(&precomputed.fat_bytes)?;
    cw.write_all(&precomputed.fat_bytes)?;

    write_dir_entries(&mut cw, precomputed)?;

    for (i, reader) in readers.iter_mut().enumerate() {
        let declared_size = precomputed
            .cluster_map
            .file_sizes
            .get(i)
            .copied()
            .unwrap_or(0);
        stream_reader(&mut cw, reader, declared_size)?;
        let pad = cluster_bytes.wrapping_sub(declared_size.rem_euclid(cluster_bytes));
        if pad != cluster_bytes {
            dir::write_zeros(&mut cw, pad)?;
        }
    }

    let written = cw.written;
    if written < precomputed.image_size {
        dir::write_zeros(&mut cw, precomputed.image_size.wrapping_sub(written))?;
    }

    Ok(())
}

/// Formats a writable target as an empty FAT volume.
///
/// # Errors
///
/// Returns `Error::Fat` when layout computation fails, or
/// `Error::Io` when writing the empty volume fails.
pub fn format<W: Write>(writer: &mut W, size: u64) -> Result<()> {
    let files: &[FileMeta<'_>] = &[];
    let precomputed = precompute(files, size)?;
    let readers: &mut [std::io::Empty] = &mut [];

    build(&precomputed, readers, writer)
}

fn write_boot_sector<W: Write>(writer: &mut W, layout: &FatLayout) -> Result<()> {
    boot::write_boot32(writer, layout)?;

    boot::write_fsinfo(writer)
}

fn write_reserved_padding<W: Write>(writer: &mut W, layout: &FatLayout) -> Result<()> {
    let extra_reserved = layout
        .reserved_sectors
        .wrapping_sub(2)
        .wrapping_mul(SECTOR_SIZE);
    if extra_reserved > 0 {
        dir::write_zeros(writer, extra_reserved)?;
    }

    Ok(())
}

fn write_dir_entries<W: Write>(writer: &mut W, precomputed: &Precomputed) -> Result<()> {
    let cluster_bytes = precomputed.layout.spc.wrapping_mul(SECTOR_SIZE);

    for i in 0..precomputed.dirs.len() {
        let dir_data = precomputed
            .dir_data
            .get(i)
            .map_or(&[][..], |dir_bytes| dir_bytes.as_slice());
        writer.write_all(dir_data)?;
        let dir_len = u64::try_from(dir_data.len()).unwrap_or(u64::MAX);
        let clusters = u64::from(
            precomputed
                .cluster_map
                .dir_counts
                .get(i)
                .copied()
                .unwrap_or(1),
        );
        let want = clusters.wrapping_mul(cluster_bytes);
        let pad = want.saturating_sub(dir_len);
        if pad > 0 {
            dir::write_zeros(writer, pad)?;
        }
    }

    Ok(())
}

fn build_all_dir_data(
    files: &[FileMeta<'_>],
    dirs: &[String],
    map: &ClusterMap,
    layout: &FatLayout,
) -> Vec<Vec<u8>> {
    dirs.iter()
        .enumerate()
        .map(|(i, _)| dir::build_data(files, dirs, map, i, layout))
        .collect()
}

fn stream_reader<W: Write>(writer: &mut W, reader: &mut impl Read, size: u64) -> Result<()> {
    let mut buf = [0_u8; 8192];
    let buf_len = u64::try_from(buf.len()).unwrap_or(u64::MAX);
    let mut rem = size;
    while rem > 0 {
        let chunk = rem.min(buf_len);
        let n = usize::try_from(chunk).unwrap_or(buf.len());
        let read = reader
            .read(buf.get_mut(..n).unwrap_or(&mut []))
            .map_err(FatError::Io)?;
        if read == 0 {
            return Err(FatError::Fat(format!(
                "reader EOF before declared size: {size} bytes expected, {rem} remaining"
            )));
        }
        writer.write_all(buf.get(..read).unwrap_or(&[]))?;
        rem = rem.wrapping_sub(u64::try_from(read).unwrap_or(u64::MAX));
    }

    Ok(())
}

fn compute_layout(image_size: u64) -> Result<FatLayout> {
    if image_size > MAX_IMAGE_SIZE {
        return Err(FatError::Fat(format!(
            "image too large for FAT32: {image_size} bytes > {MAX_IMAGE_SIZE}"
        )));
    }
    let total_sectors = image_size.div_euclid(SECTOR_SIZE);
    if total_sectors < 2 {
        return Err(FatError::Fat("image too small for reserved area".into()));
    }
    let rsvd = RESERVED_SECTORS;
    let spc_values: &[u64] = &[64, 32, 16, 8, 4, 2, 1];
    for &spc in spc_values {
        let result = test_spc(spc, total_sectors, rsvd, 0);
        let (fat_sectors, final_clusters, _) = match result {
            Some(triple) if triple.1 >= FAT32_MIN_CLUSTERS => triple,
            _ => continue,
        };
        return Ok(FatLayout {
            total_sectors,
            reserved_sectors: rsvd,
            fat_sectors,
            spc,
            data_cluster_count: final_clusters,
        });
    }

    Err(FatError::Fat(
        "image size insufficient for any FAT type".into(),
    ))
}

fn test_spc(spc: u64, total_sectors: u64, rsvd: u64, root_secs: u64) -> Option<(u64, u64, u64)> {
    let data_sectors = total_sectors.wrapping_sub(rsvd);
    let total_clusters = data_sectors.div_euclid(spc);
    if total_clusters == 0 {
        return None;
    }
    let fat_entries = total_clusters.saturating_add(2);
    let fat_bytes = fat_entries.checked_mul(FAT_ENTRY_SIZE)?;
    let fat_sectors = fat_bytes
        .next_multiple_of(SECTOR_SIZE)
        .div_euclid(SECTOR_SIZE);
    let actual_data_sectors = data_sectors
        .wrapping_sub(fat_sectors.wrapping_mul(FAT_COUNT))
        .wrapping_sub(root_secs);
    let final_clusters = actual_data_sectors.div_euclid(spc);
    if final_clusters == 0 {
        return None;
    }

    Some((fat_sectors, final_clusters, spc))
}

fn collect_dir_paths(files: &[FileMeta<'_>]) -> Vec<String> {
    let mut dirs: Vec<String> = Vec::new();
    dirs.push(String::new());
    for file in files {
        let target = std::path::Path::new(file.path);
        push_parent_dirs(&mut dirs, target);
    }
    dirs.sort_by(|left, right| left.len().cmp(&right.len()).then(left.cmp(right)));

    dirs
}

fn push_parent_dirs(dirs: &mut Vec<String>, target: &std::path::Path) {
    let mut dir = target;
    while let Some(parent) = dir.parent() {
        let dir_path = parent.to_string_lossy().into_owned();
        if !dirs.contains(&dir_path) {
            dirs.push(dir_path);
        }
        dir = parent;
    }
}

fn assign_clusters(
    files: &[FileMeta<'_>],
    dirs: &[String],
    layout: &FatLayout,
    dir_sizes: &[u64],
) -> Result<ClusterMap> {
    let cluster_bytes = layout.spc.wrapping_mul(SECTOR_SIZE);
    let mut next_cluster = u64::from(ROOT_CLUSTER);
    let mut dir_starts = Vec::with_capacity(dirs.len());
    let mut dir_counts = Vec::with_capacity(dirs.len());
    for &size in dir_sizes {
        let count = size.div_ceil(cluster_bytes);
        let start = u32::try_from(next_cluster)
            .map_err(|_conv| FatError::Fat("cluster index exceeds FAT32 range".into()))?;
        dir_starts.push(start);
        dir_counts
            .push(u32::try_from(count).map_err(|_conv| FatError::Fat("too many clusters".into()))?);
        next_cluster = next_cluster
            .checked_add(count)
            .ok_or_else(|| FatError::Fat("cluster overflow".into()))?;
    }
    let mut file_starts = Vec::with_capacity(files.len());
    let mut file_counts = Vec::with_capacity(files.len());
    let mut file_sizes = Vec::with_capacity(files.len());
    for file in files {
        if file.size > u64::from(u32::MAX) {
            return Err(FatError::Fat(format!(
                "file too large for FAT32 directory entry: {} bytes > {}",
                file.size,
                u32::MAX
            )));
        }
        let count = file.size.div_ceil(cluster_bytes);
        let start = u32::try_from(next_cluster)
            .map_err(|_conv| FatError::Fat("cluster index exceeds FAT32 range".into()))?;
        file_starts.push(start);
        file_counts.push(count);
        file_sizes.push(file.size);
        next_cluster = next_cluster
            .checked_add(count)
            .ok_or_else(|| FatError::Fat("cluster overflow".into()))?;
    }
    let used = next_cluster.wrapping_sub(u64::from(ROOT_CLUSTER));
    if used > layout.data_cluster_count {
        return Err(FatError::Fat(format!(
            "need {used} data clusters but only {} available",
            layout.data_cluster_count
        )));
    }

    Ok(ClusterMap {
        dir_starts,
        dir_counts,
        file_starts,
        file_counts,
        file_sizes,
    })
}

struct CountWriter<W: Write> {
    inner: W,
    written: u64,
}

impl<W: Write> Write for CountWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.written = self
            .written
            .wrapping_add(u64::try_from(n).unwrap_or(u64::MAX));

        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::types::{FAT32_EOC, MIN_IMAGE_SIZE, SECTOR_SIZE};

    fn read_u16_le(buf: &[u8], off: usize) -> u16 {
        let bytes = buf.get(off..off.wrapping_add(2)).unwrap_or(&[0, 0]);

        u16::from_le_bytes(bytes.try_into().unwrap_or([0, 0]))
    }

    fn read_u32_le(buf: &[u8], off: usize) -> u32 {
        let bytes = buf.get(off..off.wrapping_add(4)).unwrap_or(&[0, 0, 0, 0]);

        u32::from_le_bytes(bytes.try_into().unwrap_or([0, 0, 0, 0]))
    }

    fn read_u8(buf: &[u8], off: usize) -> u8 {
        buf.get(off).copied().unwrap_or(0)
    }

    fn bpb_is_fat32(img: &[u8]) -> bool {
        read_u16_le(img, 22) == 0
    }

    fn data_region_offset(img: &[u8]) -> usize {
        let reserved = u64::from(read_u16_le(img, 14));
        let fat_sectors = if bpb_is_fat32(img) {
            u64::from(read_u32_le(img, 36))
        } else {
            u64::from(read_u16_le(img, 22))
        };
        let bps = u64::from(read_u16_le(img, 11));
        let root_sectors = if bpb_is_fat32(img) {
            0
        } else {
            let root_entries = u64::from(read_u16_le(img, 17));
            root_entries.wrapping_mul(32).div_euclid(bps)
        };

        usize::try_from(
            reserved
                .wrapping_add(fat_sectors.wrapping_mul(2))
                .wrapping_add(root_sectors)
                .wrapping_mul(bps),
        )
        .unwrap_or(0)
    }

    fn cluster_data_offset(img: &[u8], cluster: u32) -> usize {
        let data_start = data_region_offset(img);
        let spc = u64::from(read_u8(img, 13));
        let bps = u64::from(read_u16_le(img, 11));
        let cluster_bytes = spc.wrapping_mul(bps);
        data_start.wrapping_add(
            usize::try_from(
                u64::from(cluster)
                    .wrapping_sub(2)
                    .wrapping_mul(cluster_bytes),
            )
            .unwrap_or(0),
        )
    }

    fn dir_entry_name(entry: &[u8]) -> String {
        let attr = entry.get(11).copied().unwrap_or(0);
        if attr == 0x0F {
            return String::new();
        }
        let name_bytes = entry.get(..11).unwrap_or(&[0; 11]);
        let name = core::str::from_utf8(name_bytes.get(..8).unwrap_or(&[]))
            .unwrap_or("")
            .trim_end()
            .to_owned();
        let ext = core::str::from_utf8(name_bytes.get(8..11).unwrap_or(&[]))
            .unwrap_or("")
            .trim_end()
            .to_owned();
        if ext.is_empty() {
            name
        } else {
            format!("{name}.{ext}")
        }
    }

    fn root_dir_offset(img: &[u8]) -> usize {
        let reserved = u64::from(read_u16_le(img, 14));
        let fat_sectors = if read_u16_le(img, 22) == 0 {
            u64::from(read_u32_le(img, 36))
        } else {
            u64::from(read_u16_le(img, 22))
        };
        let bps = u64::from(read_u16_le(img, 11));
        usize::try_from(
            reserved
                .wrapping_add(fat_sectors.wrapping_mul(2))
                .wrapping_mul(bps),
        )
        .unwrap_or(0)
    }

    fn find_in_dir(img: &[u8], cluster: u32, target: &str) -> Option<(u32, u32)> {
        if cluster == 0 {
            let root_dir_start = root_dir_offset(img);
            let root_entries = u64::from(read_u16_le(img, 17));
            let root_dir_bytes = usize::try_from(root_entries.wrapping_mul(32)).unwrap_or(0);
            let data = img
                .get(root_dir_start..root_dir_start.wrapping_add(root_dir_bytes))
                .unwrap_or(&[]);
            return find_in_data(data, target);
        }
        let off = cluster_data_offset(img, cluster);
        let spc = u64::from(read_u8(img, 13));
        let bps = u64::from(read_u16_le(img, 11));
        let cluster_bytes = usize::try_from(spc.wrapping_mul(bps)).unwrap_or(0);
        let data = img.get(off..off.wrapping_add(cluster_bytes)).unwrap_or(&[]);

        find_in_data(data, target)
    }

    fn try_match_entry(entry: &[u8], target: &str) -> Option<(u32, u32)> {
        if entry.get(11).copied().unwrap_or(0) == 0x0F {
            return None;
        }
        if dir_entry_name(entry) != target {
            return None;
        }
        let hi = u32::from(read_u16_le(entry, 20));
        let lo = u32::from(read_u16_le(entry, 26));
        let cluster = (hi << 16) | lo;
        let size = read_u32_le(entry, 28);

        Some((cluster, size))
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "while + if inside mod tests exceeds nesting threshold of 3"
    )]
    fn find_in_data(data: &[u8], target: &str) -> Option<(u32, u32)> {
        let target_upper = target.to_uppercase();
        let mut off = 0_usize;
        while off.wrapping_add(32) <= data.len() {
            let entry = data.get(off..off.wrapping_add(32))?;
            let first = entry.first().copied().unwrap_or(0);
            if first == 0 || first == 0xE5 {
                off = off.wrapping_add(32);
                continue;
            }
            if let Some(result) = try_match_entry(entry, &target_upper) {
                return Some(result);
            }
            off = off.wrapping_add(32);
        }

        None
    }

    fn cluster_data_slice(img: &[u8], cluster: u32) -> &[u8] {
        let off = cluster_data_offset(img, cluster);
        let spc = u64::from(read_u8(img, 13));
        let bps = u64::from(read_u16_le(img, 11));
        let cluster_bytes = usize::try_from(spc.wrapping_mul(bps)).unwrap_or(0);

        img.get(off..off.wrapping_add(cluster_bytes)).unwrap_or(&[])
    }

    fn build_image(files: &[(&str, &[u8])], image_size: u64) -> Vec<u8> {
        let metas: Vec<FileMeta<'_>> = files
            .iter()
            .map(|&(path, data)| FileMeta::new(path, u64::try_from(data.len()).unwrap_or(0)))
            .collect();
        let precomputed = precompute(&metas, image_size).expect("precompute must succeed");
        let mut readers: Vec<Cursor<&[u8]>> = files
            .iter()
            .map(|&(_path, data)| Cursor::new(data))
            .collect();
        let mut out = Vec::new();
        build(&precomputed, &mut readers, &mut out).expect("build must succeed");
        out
    }

    #[test]
    fn format_produces_bootable_image() {
        // ARRANGE
        let mut buf = Vec::new();

        // ACT
        format(&mut buf, 36 * 1024 * 1024).expect("format must succeed");

        // ASSERT
        assert_eq!(
            buf.get(510..512),
            Some(&[0x55, 0xAA][..]),
            "boot signature must be valid"
        );
        assert!(
            buf.len() >= 36 * 1024 * 1024,
            "image must be at least 36 MiB"
        );
    }

    #[test]
    fn format_boot_sector_fields() {
        // ARRANGE
        let mut buf = Vec::new();

        // ACT
        format(&mut buf, 36 * 1024 * 1024).expect("format must succeed");

        // ASSERT
        assert_eq!(
            buf.get(0..3),
            Some(&[0xEB, 0x58, 0x90][..]),
            "jump instruction"
        );
        assert_eq!(buf.get(3..11), Some(&b"MSWIN4.1"[..]), "OEM ID");
        assert_eq!(buf.get(71..82), Some(&b"EFI        "[..]), "volume label");
    }

    #[test]
    fn build_produces_valid_image() {
        // ARRANGE
        let files = &[("EFI/BOOT/BOOTX64.EFI", b"uki-payload".as_slice())];

        // ACT
        let out = build_image(files, 36 * 1024 * 1024);

        // ASSERT
        assert_eq!(out.get(510..512), Some(&[0x55, 0xAA][..]), "boot signature");
    }

    #[test]
    fn build_multiple_files() {
        // ARRANGE
        let files = &[
            ("EFI/BOOT/BOOTX64.EFI", b"uki".as_slice()),
            ("cfg.txt", b"config".as_slice()),
        ];

        // ACT
        let out = build_image(files, 36 * 1024 * 1024);

        // ASSERT
        assert!(!out.is_empty());
    }

    #[test]
    fn build_nested_directories() {
        // ARRANGE
        let files = &[
            ("EFI/BOOT/BOOTX64.EFI", b"uki".as_slice()),
            ("overlays/rpi/config.txt", b"arm_64bit=1".as_slice()),
        ];

        // ACT
        let out = build_image(files, 36 * 1024 * 1024);

        // ASSERT
        assert!(!out.is_empty());
    }

    #[test]
    fn build_chains_root_directory_across_multiple_clusters() {
        // ARRANGE
        let names: Vec<String> = (0..200).map(|i| format!("file{i:03}.bin")).collect();
        let files: Vec<(&str, &[u8])> = names.iter().map(|name| (name.as_str(), &[][..])).collect();
        let image_size = MIN_IMAGE_SIZE.wrapping_add(1024 * 1024);

        // ACT
        let out = build_image(&files, image_size);

        // ASSERT
        let fat_offset = usize::try_from(8_u64.wrapping_mul(SECTOR_SIZE)).unwrap_or(4096);
        let root_next = read_u32_le(&out, fat_offset + 2 * 4);
        assert_ne!(
            root_next, FAT32_EOC,
            "root directory must span more than one cluster"
        );
        assert_eq!(
            root_next, 3,
            "root directory must chain from cluster 2 to 3"
        );
    }

    #[test]
    fn image_has_boot_file_at_expected_path() {
        // ARRANGE
        let payload = b"uki-binary-data-1234";
        let files = &[("EFI/BOOT/BOOTX64.EFI", payload.as_slice())];

        // ACT
        let out = build_image(files, 36 * 1024 * 1024);

        // ASSERT
        let bps = u64::from(read_u16_le(&out, 11));
        assert_eq!(bps, SECTOR_SIZE, "bytes per sector must match");
        let is_32 = bpb_is_fat32(&out);
        let root_cluster = if is_32 { read_u32_le(&out, 44) } else { 0 };
        let (efi_cluster, _) =
            find_in_dir(&out, root_cluster, "EFI").expect("must find EFI directory");
        let (boot_cluster, _) =
            find_in_dir(&out, efi_cluster, "BOOT").expect("must find BOOT directory in EFI");
        let (file_cluster, file_size) = find_in_dir(&out, boot_cluster, "BOOTX64.EFI")
            .expect("must find BOOTX64.EFI in EFI/BOOT");
        assert_eq!(
            file_size,
            u32::try_from(payload.len()).unwrap_or(0),
            "file size must match"
        );
        let file_data = cluster_data_slice(&out, file_cluster);
        let read_size = usize::try_from(file_size).unwrap_or(0);
        assert_eq!(
            file_data.get(..read_size),
            Some(&payload[..]),
            "file content must match"
        );
    }

    #[test]
    fn image_with_nested_overlay_files() {
        // ARRANGE
        let files = &[
            ("EFI/BOOT/BOOTX64.EFI", b"uki".as_slice()),
            ("overlays/rpi/config.txt", b"arm_64bit=1".as_slice()),
        ];

        // ACT
        let out = build_image(files, 36 * 1024 * 1024);

        // ASSERT
        let is_32 = bpb_is_fat32(&out);
        let root_cluster = if is_32 { read_u32_le(&out, 44) } else { 0 };
        let (overlays_cluster, _) =
            find_in_dir(&out, root_cluster, "overlays").expect("must find overlays directory");
        let (rpi_cluster, _) =
            find_in_dir(&out, overlays_cluster, "rpi").expect("must find rpi directory");
        let (_cfg_cluster, _) = find_in_dir(&out, rpi_cluster, "config.txt")
            .expect("must find config.txt in overlays/rpi");
    }

    #[test]
    fn fat32_used_for_large_volumes() {
        // ARRANGE
        let data = vec![0xAB_u8; 300 * 1024 * 1024];
        let files = &[("EFI/BOOT/BOOTX64.EFI", data.as_slice())];

        // ACT
        let out = build_image(files, 400 * 1024 * 1024);

        // ASSERT
        assert!(
            bpb_is_fat32(&out),
            "large volume must be FAT32, got: FATSz16={}, FATSz32={}",
            read_u16_le(&out, 22),
            read_u32_le(&out, 36)
        );
        assert!(
            read_u32_le(&out, 36) > 0,
            "FATSz32 must be non-zero for FAT32"
        );
        assert_eq!(
            read_u32_le(&out, 44),
            ROOT_CLUSTER,
            "RootClus must be 2 for FAT32"
        );
        let (efi_cluster, _) = find_in_dir(&out, ROOT_CLUSTER, "EFI")
            .expect("must find EFI directory in root for large volume");
        assert!(efi_cluster >= ROOT_CLUSTER);
        let (boot_cluster, _) = find_in_dir(&out, efi_cluster, "BOOT")
            .expect("must find BOOT directory in EFI for large volume");
        let (_file_cluster, file_size) = find_in_dir(&out, boot_cluster, "BOOTX64.EFI")
            .expect("must find BOOTX64.EFI in EFI/BOOT for large volume");
        assert_eq!(
            u64::from(file_size),
            u64::try_from(data.len()).unwrap_or(u64::MAX),
            "file size must match in FAT32"
        );
    }

    #[test]
    fn build_detects_short_reader() {
        // ARRANGE
        let files = &[FileMeta::new("test.bin", 16)];
        let precomputed = precompute(files, 36 * 1024 * 1024).expect("precompute must succeed");
        let mut empty_reader = std::io::empty();

        // ACT
        let mut out = Vec::new();
        let result = build(
            &precomputed,
            core::slice::from_mut(&mut empty_reader),
            &mut out,
        );

        // ASSERT
        assert!(result.is_err(), "short reader must produce an error");
    }

    #[test]
    fn build_with_large_payload() {
        // ARRANGE
        let data = vec![0xAB_u8; 100_000];
        let files = &[("EFI/BOOT/BOOTX64.EFI", data.as_slice())];

        // ACT
        let out = build_image(files, 36 * 1024 * 1024);

        // ASSERT
        assert_eq!(
            u64::try_from(out.len()).unwrap_or(u64::MAX),
            36 * 1024 * 1024
        );
    }

    #[test]
    fn rejected_too_small_image() {
        // ARRANGE
        let files = &[FileMeta::new("test.bin", 100_000)];

        // ACT
        let err = precompute(files, 512);

        // ASSERT
        assert!(err.is_err(), "too-small image must be rejected");
    }

    #[test]
    fn min_image_size_is_the_smallest_acceptable_size() {
        // ARRANGE
        let files: &[FileMeta<'_>] = &[];

        // ACT
        let accepted = precompute(files, MIN_IMAGE_SIZE).is_ok();
        let rejected = precompute(files, MIN_IMAGE_SIZE.saturating_sub(1)).is_ok();

        // ASSERT
        assert!(accepted, "minimum image size must be accepted");
        assert!(!rejected, "one byte below minimum must be rejected");
    }

    #[test]
    fn compute_layout_accepts_the_largest_fat32_volume() {
        // ARRANGE / ACT
        let result = compute_layout(MAX_IMAGE_SIZE);

        // ASSERT
        assert!(result.is_ok(), "the largest FAT32 volume must be accepted");
    }

    #[test]
    fn compute_layout_rejects_volumes_above_fat32_ceiling() {
        // ARRANGE
        let oversized = MAX_IMAGE_SIZE.saturating_add(SECTOR_SIZE);

        // ACT
        let result = compute_layout(oversized);

        // ASSERT
        assert!(
            result.is_err(),
            "volumes above the FAT32 ceiling must be rejected"
        );
    }

    #[test]
    fn precompute_rejects_files_larger_than_fat32_dir_entry() {
        // ARRANGE
        let files = &[FileMeta::new(
            "big.bin",
            u64::from(u32::MAX).saturating_add(1),
        )];

        // ACT
        let result = precompute(files, 5_u64 * 1024 * 1024 * 1024);

        // ASSERT
        assert!(
            result.is_err(),
            "files over 4 GiB must not fit in a FAT32 directory entry"
        );
    }
}
