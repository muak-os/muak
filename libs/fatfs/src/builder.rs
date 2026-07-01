//! FAT image building API.

use std::io::Write;

use crate::boot;
use crate::dir;
use crate::error::FatError;
use crate::error::Result;
use crate::table;
use crate::types::{
    ClusterMap, FAT_COUNT, FAT16_MIN_CLUSTERS, FAT32_MIN_CLUSTERS, FatKind, FatLayout, FileSource,
    ROOT_CLUSTER, SECTOR_SIZE, fat12_16_cluster, fat32_cluster,
};

/// Builds a FAT image with the given files, size, and writer.
///
/// # Errors
///
/// Returns `FatError::Fat` when layout computation fails or files don't fit,
/// or `FatError::Io` when writing fails.
pub fn build<T: FileSource, W: Write>(
    files: &mut [T],
    image_size: u64,
    writer: &mut W,
) -> Result<()> {
    let layout = compute_layout(image_size)?;
    let dirs = collect_dir_paths(files);
    let map = assign_clusters(files, &dirs, &layout)?;

    let mut cw = CountWriter {
        inner: writer,
        written: 0,
    };
    let cluster_bytes = layout.spc.wrapping_mul(SECTOR_SIZE);

    match layout.kind {
        FatKind::Fat32 => {
            boot::write_boot32(&mut cw, &layout)?;
            boot::write_fsinfo(&mut cw)?;
        }
        FatKind::Fat12 | FatKind::Fat16 => {
            boot::write_boot12_16(&mut cw, &layout)?;
        }
    }

    let sectors_written_for_boot = match layout.kind {
        FatKind::Fat32 => 2_u64,
        FatKind::Fat12 | FatKind::Fat16 => 1_u64,
    };
    let extra_reserved = layout
        .reserved_sectors
        .wrapping_sub(sectors_written_for_boot)
        .wrapping_mul(SECTOR_SIZE);
    if extra_reserved > 0 {
        dir::write_zeros(&mut cw, extra_reserved)?;
    }

    let fat = table::make_fat(&map, &layout);
    cw.write_all(&fat)?;
    cw.write_all(&fat)?;

    match layout.kind {
        FatKind::Fat12 | FatKind::Fat16 => {
            let root_dir_data = dir::build_data(files, &dirs, &map, 0, &layout);
            cw.write_all(&root_dir_data)?;
            let root_size = layout.root_dir_sectors.wrapping_mul(SECTOR_SIZE);
            let remaining =
                root_size.wrapping_sub(u64::try_from(root_dir_data.len()).unwrap_or(u64::MAX));
            if remaining > 0 && remaining < root_size {
                dir::write_zeros(&mut cw, remaining)?;
            }
        }
        FatKind::Fat32 => {}
    }

    for i in 0..dirs.len() {
        if i == 0 && layout.kind != FatKind::Fat32 {
            continue;
        }
        let dir_data = dir::build_data(files, &dirs, &map, i, &layout);
        cw.write_all(&dir_data)?;
        let dir_len = u64::try_from(dir_data.len()).unwrap_or(u64::MAX);
        let pad = cluster_bytes.wrapping_sub(dir_len.rem_euclid(cluster_bytes));
        if pad != cluster_bytes {
            dir::write_zeros(&mut cw, pad)?;
        }
    }

    for file in files.iter_mut() {
        stream_file(&mut cw, file)?;
        let pad = cluster_bytes.wrapping_sub(file.size().rem_euclid(cluster_bytes));
        if pad != cluster_bytes {
            dir::write_zeros(&mut cw, pad)?;
        }
    }

    let written = cw.written;
    if written < image_size {
        dir::write_zeros(&mut cw, image_size.wrapping_sub(written))?;
    }

    Ok(())
}

/// Formats a writable target as a FAT volume.
///
/// # Errors
///
/// Returns `FatError::Fat` when layout computation fails, or
/// `FatError::Io` when writing the empty volume fails.
pub fn format<W: Write>(writer: &mut W, size: u64) -> Result<()> {
    struct NeverSource;
    impl FileSource for NeverSource {
        fn path(&self) -> &'static str {
            ""
        }

        fn size(&self) -> u64 {
            0
        }

        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Ok(0)
        }
    }
    let files: &mut [NeverSource] = &mut [];

    build(files, size, writer)
}

fn compute_layout(image_size: u64) -> Result<FatLayout> {
    let total_sectors = image_size.div_euclid(SECTOR_SIZE);
    if total_sectors < 2 {
        return Err(FatError::Fat("image too small for reserved area".into()));
    }
    let root_dir_sectors = 512_u64.wrapping_mul(32).div_euclid(SECTOR_SIZE);
    let spc_values: &[u64] = &[64, 32, 16, 8, 4, 2, 1];
    for &(rsvd, kind) in &[
        (32_u64, FatKind::Fat32),
        (1, FatKind::Fat16),
        (1, FatKind::Fat12),
    ] {
        if total_sectors <= rsvd {
            continue;
        }
        let root_secs = match kind {
            FatKind::Fat32 => 0,
            FatKind::Fat12 | FatKind::Fat16 => root_dir_sectors,
        };
        let valid_spcs: &[u64] = match kind {
            FatKind::Fat32 => &[64, 32, 16, 8],
            FatKind::Fat12 | FatKind::Fat16 => spc_values,
        };
        for &spc in valid_spcs {
            let result = test_spc(spc, total_sectors, rsvd, root_secs);
            let (fat_sectors, final_clusters, _) = match result {
                Some(triple) if check_kind(triple.1, kind) => triple,
                _ => continue,
            };
            return Ok(FatLayout {
                total_sectors,
                reserved_sectors: rsvd,
                fat_sectors,
                spc,
                root_dir_sectors: root_secs,
                data_cluster_count: final_clusters,
                kind,
            });
        }
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
    let fat_bytes = fat_entries.checked_mul(4)?;
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

fn check_kind(final_clusters: u64, kind: FatKind) -> bool {
    match kind {
        FatKind::Fat12 => final_clusters < FAT16_MIN_CLUSTERS,
        FatKind::Fat16 => (FAT16_MIN_CLUSTERS..FAT32_MIN_CLUSTERS).contains(&final_clusters),
        FatKind::Fat32 => final_clusters >= FAT32_MIN_CLUSTERS,
    }
}

fn collect_dir_paths(files: &[impl FileSource]) -> Vec<String> {
    let mut dirs: Vec<String> = Vec::new();
    dirs.push(String::new());
    for ef in files {
        let target = std::path::Path::new(ef.path());
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
    files: &[impl FileSource],
    dirs: &[String],
    layout: &FatLayout,
) -> Result<ClusterMap> {
    let is_fat32 = layout.kind == FatKind::Fat32;
    let dir_clusters: Vec<u32> = if is_fat32 {
        (0..dirs.len()).map(fat32_cluster).collect()
    } else {
        (0..dirs.len()).map(fat12_16_cluster).collect()
    };
    let dir_data_count = if is_fat32 {
        dirs.len()
    } else {
        dirs.len().saturating_sub(1)
    };
    let dir_count =
        u64::try_from(dir_data_count).map_err(|_conv| FatError::Fat("too many dirs".into()))?;
    let mut next_cluster = u64::from(ROOT_CLUSTER).wrapping_add(dir_count);
    let mut file_starts = Vec::with_capacity(files.len());
    let mut file_counts = Vec::with_capacity(files.len());
    let cluster_bytes = layout.spc.wrapping_mul(SECTOR_SIZE);
    for ef in files {
        let count = ef.size().div_ceil(cluster_bytes);
        file_starts.push(u32::try_from(next_cluster).unwrap_or(u32::MAX));
        file_counts.push(count);
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
        dir_clusters,
        file_starts,
        file_counts,
    })
}

fn stream_file<W: Write>(writer: &mut W, file: &mut impl FileSource) -> Result<()> {
    let mut buf = [0_u8; 8192];
    let buf_len = u64::try_from(buf.len()).unwrap_or(u64::MAX);
    let mut rem = file.size();
    while rem > 0 {
        let chunk = rem.min(buf_len);
        let n = usize::try_from(chunk).unwrap_or(buf.len());
        let read = file
            .read(buf.get_mut(..n).unwrap_or(&mut []))
            .map_err(FatError::Io)?;
        if read == 0 {
            return Err(FatError::Fat(format!(
                "reader EOF before declared size: {}",
                file.path()
            )));
        }
        writer.write_all(buf.get(..read).unwrap_or(&[]))?;
        rem = rem.wrapping_sub(u64::try_from(read).unwrap_or(u64::MAX));
    }

    Ok(())
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
    use super::*;
    use crate::types::SECTOR_SIZE;

    struct TestFile {
        path: String,
        size: u64,
        data: Vec<u8>,
        pos: usize,
    }

    impl TestFile {
        fn boot(data: &[u8]) -> Self {
            Self {
                path: "EFI/BOOT/BOOTX64.EFI".into(),
                size: u64::try_from(data.len()).unwrap_or(0),
                data: data.to_vec(),
                pos: 0,
            }
        }

        fn overlay(path: &str, data: &[u8]) -> Self {
            Self {
                path: path.into(),
                size: u64::try_from(data.len()).unwrap_or(0),
                data: data.to_vec(),
                pos: 0,
            }
        }
    }

    impl FileSource for TestFile {
        fn path(&self) -> &str {
            &self.path
        }

        fn size(&self) -> u64 {
            self.size
        }

        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let remaining = self.data.len().wrapping_sub(self.pos);
            let to_read = buf.len().min(remaining);
            let end = self.pos.wrapping_add(to_read);
            let chunk = self.data.get(self.pos..end).unwrap_or(&[]);
            let dst = buf.get_mut(..to_read).unwrap_or(&mut []);
            let copy_len = dst.len().min(chunk.len());
            let chunk_part = chunk.get(..copy_len).unwrap_or(&[]);
            let dst_part = dst.get_mut(..copy_len).unwrap_or(&mut []);
            dst_part.copy_from_slice(chunk_part);
            self.pos = self.pos.wrapping_add(to_read);

            Ok(to_read)
        }
    }

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

    #[test]
    fn format_produces_bootable_image() {
        // ARRANGE
        let mut buf = Vec::new();

        // ACT
        format(&mut buf, 1024 * 1024).expect("format must succeed");

        // ASSERT
        assert_eq!(
            buf.get(510..512),
            Some(&[0x55, 0xAA][..]),
            "boot signature must be valid"
        );
        assert!(buf.len() >= 1024 * 1024, "image must be at least 1 MiB");
    }

    #[test]
    fn format_boot_sector_fields() {
        // ARRANGE
        let mut buf = Vec::new();

        // ACT
        format(&mut buf, 1024 * 1024).expect("format must succeed");

        // ASSERT
        let jump = buf.get(0..3).unwrap_or(&[]);
        assert!(
            jump == [0xEB, 0x58, 0x90] || jump == [0xEB, 0x3C, 0x90],
            "jump instruction must be valid {jump:?}"
        );
        assert_eq!(buf.get(3..11), Some(&b"MSWIN4.1"[..]), "OEM ID");
        assert_eq!(buf.get(43..54), Some(&b"EFI        "[..]), "volume label");
    }

    #[test]
    fn build_produces_valid_image() {
        // ARRANGE
        let mut files = vec![TestFile::boot(b"uki-payload")];
        let size = 1024 * 1024;

        // ACT
        let mut out = Vec::new();
        build(&mut files, size, &mut out).expect("build must succeed");

        // ASSERT
        assert_eq!(out.get(510..512), Some(&[0x55, 0xAA][..]), "boot signature");
    }

    #[test]
    fn build_multiple_files() {
        // ARRANGE
        let mut files = vec![
            TestFile::boot(b"uki"),
            TestFile::overlay("cfg.txt", b"config"),
        ];
        let size = 1024 * 1024;

        // ACT
        let mut out = Vec::new();
        build(&mut files, size, &mut out).expect("build must succeed");

        // ASSERT
        assert!(!out.is_empty());
    }

    #[test]
    fn build_nested_directories() {
        // ARRANGE
        let mut files = vec![
            TestFile::boot(b"uki"),
            TestFile::overlay("overlays/rpi/config.txt", b"arm_64bit=1"),
        ];
        let size = 1024 * 1024;

        // ACT
        let mut out = Vec::new();
        build(&mut files, size, &mut out).expect("build must succeed");

        // ASSERT
        assert!(!out.is_empty());
    }

    #[test]
    fn image_has_boot_file_at_expected_path() {
        // ARRANGE
        let payload = b"uki-binary-data-1234";
        let mut files = vec![TestFile::boot(payload)];
        let size = 1024 * 1024;

        // ACT
        let mut out = Vec::new();
        build(&mut files, size, &mut out).expect("build must succeed");

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
        let mut files = vec![
            TestFile::boot(b"uki"),
            TestFile::overlay("overlays/rpi/config.txt", b"arm_64bit=1"),
        ];
        let size = 1024 * 1024;

        // ACT
        let mut out = Vec::new();
        build(&mut files, size, &mut out).expect("build must succeed");

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
    fn fat16_bpb_for_small_volumes() {
        // ARRANGE
        let mut files = vec![TestFile::boot(b"uki")];
        let size = 1024 * 1024;

        // ACT
        let mut out = Vec::new();
        build(&mut files, size, &mut out).expect("build must succeed");

        // ASSERT
        assert!(
            !bpb_is_fat32(&out),
            "small volume must be FAT12/16 not FAT32"
        );
        let fat_sectors = u64::from(read_u16_le(&out, 22));
        assert!(fat_sectors > 0, "FATSz16 must be non-zero");
        let root_entries = read_u16_le(&out, 17);
        assert!(root_entries > 0, "RootEntCnt must be non-zero");
        let fs_type = out.get(54..62).unwrap_or(&[]);
        assert!(
            fs_type == b"FAT12   " || fs_type == b"FAT16   ",
            "file system type must be FAT12 or FAT16 at offset 54, got {fs_type:?}"
        );
    }

    #[test]
    fn fat16_root_dir_accessible() {
        // ARRANGE
        let mut files = vec![TestFile::boot(b"uki")];
        let size = 1024 * 1024;

        // ACT
        let mut out = Vec::new();
        build(&mut files, size, &mut out).expect("build must succeed");

        // ASSERT
        let is_32 = bpb_is_fat32(&out);
        assert!(!is_32, "must be FAT16 for this test");
        let (efi_cluster, _) = find_in_dir(&out, 0, "EFI").expect("must find EFI in root dir");
        assert!(
            efi_cluster >= ROOT_CLUSTER,
            "EFI dir must have valid cluster"
        );
        let (boot_cluster, _) =
            find_in_dir(&out, efi_cluster, "BOOT").expect("must find BOOT in EFI dir");
        let (file_cluster, file_size) = find_in_dir(&out, boot_cluster, "BOOTX64.EFI")
            .expect("must find BOOTX64.EFI in EFI/BOOT");
        assert!(file_size > 0);
        let file_data = cluster_data_slice(&out, file_cluster);
        assert_eq!(
            file_data.get(..3),
            Some(&b"uki"[..]),
            "file data must be correct"
        );
    }

    #[test]
    fn fat32_used_for_large_volumes() {
        // ARRANGE
        let data = vec![0xAB_u8; 300 * 1024 * 1024];
        let mut files = vec![TestFile::boot(&data)];
        let size = 400 * 1024 * 1024;

        // ACT
        let mut out = Vec::new();
        build(&mut files, size, &mut out).expect("build must succeed");

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
        let mut file = TestFile {
            path: "test.bin".into(),
            size: 16,
            data: Vec::new(),
            pos: 0,
        };

        // ACT
        let mut out = Vec::new();
        let result = build(core::slice::from_mut(&mut file), 1024 * 1024, &mut out);

        // ASSERT
        assert!(result.is_err(), "short reader must produce an error");
    }

    #[test]
    fn compute_layout_basic() {
        // ARRANGE / ACT
        let layout = compute_layout(1024 * 1024).expect("layout must compute");

        // ASSERT
        assert_eq!(layout.kind, FatKind::Fat12, "1 MiB image must be FAT12");
        assert_eq!(layout.total_sectors, 2048);
        assert!(layout.fat_sectors > 0);
    }

    #[test]
    fn build_with_large_payload() {
        // ARRANGE
        let data = vec![0xAB_u8; 100_000];
        let mut files = vec![TestFile::boot(&data)];
        let size = 1024 * 1024;

        // ACT
        let mut out = Vec::new();
        build(&mut files, size, &mut out).expect("build must succeed");

        // ASSERT
        assert_eq!(u64::try_from(out.len()).unwrap_or(u64::MAX), size);
    }

    #[test]
    fn fat16_bpb_matches_actual_layout() {
        // ARRANGE
        let data = vec![0xAB_u8; 100_000];
        let mut files = vec![TestFile::boot(&data)];
        let size = 1024 * 1024;

        // ACT
        let mut out = Vec::new();
        build(&mut files, size, &mut out).expect("build must succeed");

        // ASSERT
        let is_32 = bpb_is_fat32(&out);
        assert!(!is_32, "small volume must be FAT16");
        let fat_sectors = u64::from(read_u16_le(&out, 22));
        let bps = u64::from(read_u16_le(&out, 11));
        let reserved = u64::from(read_u16_le(&out, 14));
        let expected_fat_bytes = fat_sectors.wrapping_mul(bps);
        let fat1_start = usize::try_from(reserved.wrapping_mul(bps)).unwrap_or(0);
        let fat1_end = fat1_start.wrapping_add(usize::try_from(expected_fat_bytes).unwrap_or(0));
        let fat2_end = fat1_end.wrapping_add(usize::try_from(expected_fat_bytes).unwrap_or(0));
        assert!(
            out.len() >= fat2_end,
            "image must be large enough for two FATs"
        );
        assert_eq!(
            out.get(fat1_start..fat1_end),
            out.get(fat1_end..fat2_end),
            "FAT1 and FAT2 must be identical"
        );
    }

    #[test]
    fn rejected_too_small_image() {
        // ARRANGE
        let mut files = vec![TestFile::boot(&[0_u8; 100_000])];

        // ACT
        let mut out = Vec::new();
        let err = build(&mut files, 512, &mut out);

        // ASSERT
        assert!(err.is_err(), "too-small image must be rejected");
    }
}
