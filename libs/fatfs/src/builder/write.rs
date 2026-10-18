//! Streaming writers for the FAT image.

use std::io::{Read, Write};

use crate::boot;
use crate::dir;
use crate::error::{FatError, Result};
use crate::types::{BACKUP_BOOT_SECTOR, FatLayout, FsInfo, Precomputed, SECTOR_SIZE};

/// Writer that tracks how many bytes reached the sink.
pub(crate) struct CountWriter<W: Write> {
    pub(crate) inner: W,
    pub(crate) written: u64,
}

impl<W: Write> Write for CountWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let count = self.inner.write(buf)?;
        self.written = self
            .written
            .wrapping_add(u64::try_from(count).unwrap_or(u64::MAX));

        Ok(count)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Writes the reserved region: boot sector, `FSInfo`, padding and the boot sector backup.
pub(crate) fn write_boot_sector<W: Write>(
    writer: &mut W,
    layout: &FatLayout,
    info: FsInfo,
) -> Result<()> {
    let boot = boot::sector(layout);
    writer.write_all(&boot)?;
    writer.write_all(&boot::fsinfo(info))?;
    let before_backup = u64::from(BACKUP_BOOT_SECTOR).saturating_sub(2);
    dir::write_zeros(writer, before_backup.saturating_mul(SECTOR_SIZE))?;
    writer.write_all(&boot)?;
    let after_backup = layout
        .reserved_sectors
        .saturating_sub(u64::from(BACKUP_BOOT_SECTOR).saturating_add(1));
    dir::write_zeros(writer, after_backup.saturating_mul(SECTOR_SIZE))?;

    Ok(())
}

/// Writes every directory cluster, padded to its allocated cluster count.
pub(crate) fn write_dir_entries<W: Write>(
    writer: &mut W,
    precomputed: &Precomputed<'_>,
) -> Result<()> {
    let cluster_bytes = precomputed.layout.spc.wrapping_mul(SECTOR_SIZE);
    for dir_index in 0..precomputed.tree.len() {
        let data = precomputed
            .dir_data
            .get(dir_index)
            .map_or(&[][..], Vec::as_slice);
        writer.write_all(data)?;
        let len = u64::try_from(data.len()).unwrap_or(u64::MAX);
        let clusters = u64::from(
            precomputed
                .cluster_map
                .dir_counts
                .get(dir_index)
                .copied()
                .unwrap_or(1),
        );
        let pad = clusters.wrapping_mul(cluster_bytes).saturating_sub(len);
        if pad > 0 {
            dir::write_zeros(writer, pad)?;
        }
    }

    Ok(())
}

/// Streams exactly `size` bytes from `reader`, padded to the next cluster boundary.
pub(crate) fn stream_reader<W: Write, R: Read>(
    writer: &mut W,
    reader: &mut R,
    size: u64,
    cluster_bytes: u64,
) -> Result<()> {
    let mut buf = [0_u8; 8192];
    let buf_len = u64::try_from(buf.len()).unwrap_or(u64::MAX);
    let mut rem = size;
    while rem > 0 {
        let chunk = rem.min(buf_len);
        let wanted = usize::try_from(chunk).unwrap_or(buf.len());
        let read = reader
            .read(buf.get_mut(..wanted).unwrap_or(&mut []))
            .map_err(FatError::Io)?;
        if read == 0 {
            return Err(FatError::Fat(format!(
                "reader EOF before declared size: {size} bytes expected, {rem} remaining"
            )));
        }
        writer.write_all(buf.get(..read).unwrap_or(&[]))?;
        rem = rem.wrapping_sub(u64::try_from(read).unwrap_or(u64::MAX));
    }
    let pad = cluster_bytes.saturating_sub(size.rem_euclid(cluster_bytes));
    if pad != cluster_bytes {
        dir::write_zeros(writer, pad)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_writer_tracks_the_written_bytes() {
        // ARRANGE
        let mut sink = Vec::new();
        let mut writer = CountWriter {
            inner: &mut sink,
            written: 0,
        };

        // ACT
        writer.write_all(b"hello").expect("write must succeed");

        // ASSERT
        assert_eq!(writer.written, 5, "tracked bytes");
        assert_eq!(writer.inner.as_slice(), b"hello".as_slice(), "payload");
    }

    #[test]
    fn stream_reader_pads_to_the_cluster_boundary() {
        // ARRANGE
        let payload = b"0123456789";
        let mut reader = payload.as_slice();
        let mut out = Vec::new();

        // ACT
        stream_reader(&mut out, &mut reader, 10, 8).expect("stream must succeed");

        // ASSERT
        assert_eq!(out.len(), 16, "two clusters for ten bytes");
        assert_eq!(out.get(..10), Some(&payload[..]), "payload");
        assert_eq!(out.get(10..), Some(&[0_u8; 6][..]), "zero padding");
    }

    #[test]
    fn stream_reader_rejects_short_readers() {
        // ARRANGE
        let mut reader = b"short".as_slice();
        let mut out = Vec::new();

        // ACT
        let result = stream_reader(&mut out, &mut reader, 64, 8);

        // ASSERT
        assert!(result.is_err(), "short readers must fail the build");
    }
}
