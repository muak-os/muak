//! Deterministic single-pass chunked zstd compression for EROFS pclusters.

use std::io::Read;

use zstd::zstd_safe::{CCtx, get_error_name};

use super::model::{CompressedLayout, Pcluster};
use super::zstd::{PCLUSTER_SIZE, compression_error, new_cctx};
use crate::error::{ErofsError, Result};

const CHUNK_START: usize = 4 * PCLUSTER_SIZE;
/// Upper bound for adaptive pcluster input takes; sizes the writer staging buffer.
pub(super) const CHUNK_MAX: usize = 64 * PCLUSTER_SIZE;
const DST_LEN: usize = PCLUSTER_SIZE + 32;

/// Measure file data into layout-only pclusters without retaining bytes (pass 1).
///
/// # Errors
///
/// Returns [`ErofsError::FileReadMismatch`] when the stream length deviates from
/// `expected_size`, and compression errors on zstd failures.
pub(super) fn compress_file(
    reader: &mut dyn Read,
    expected_size: usize,
    path: &str,
    level: i32,
) -> Result<Option<CompressedLayout>> {
    if expected_size == 0 {
        return Ok(None);
    }
    let mut cctx = new_cctx(level)?;
    let mut src = vec![0_u8; CHUNK_MAX];
    let mut dst = vec![0_u8; DST_LEN];
    let mut pclusters: Vec<Pcluster> = Vec::new();
    let mut remaining = expected_size;
    let mut buffered = 0_usize;
    let mut take = CHUNK_START;

    while remaining > 0 {
        let mut want = take.min(remaining).min(CHUNK_MAX);
        if remaining > want && remaining.saturating_sub(want) < PCLUSTER_SIZE {
            want = remaining;
        }
        buffered = top_up(
            reader,
            &mut src,
            buffered,
            want,
            path,
            expected_size.saturating_sub(remaining),
            expected_size,
        )?;
        let Some(window) = src.get(..buffered) else {
            return Err(ErofsError::Internal("chunk window out of bounds"));
        };
        let Some(out_len) = compress_fitting(&mut cctx, &mut dst, window, &mut want)? else {
            return Ok(None);
        };
        pclusters.push(Pcluster {
            input_len: want,
            compressed_len: out_len,
        });
        src.copy_within(want..buffered, 0);
        buffered = buffered.saturating_sub(want);
        remaining = remaining.saturating_sub(want);
        take = PCLUSTER_SIZE
            .saturating_mul(want)
            .checked_div(out_len.max(1))
            .unwrap_or(CHUNK_MAX)
            .clamp(PCLUSTER_SIZE, CHUNK_MAX);
    }

    let mut extra = [0_u8; 1];
    let read = reader
        .read(&mut extra)
        .map_err(|err| ErofsError::Io(std::io::Error::new(err.kind(), format!("{path}: {err}"))))?;
    if read > 0 {
        return Err(ErofsError::FileReadMismatch {
            path: path.to_owned(),
            expected: expected_size,
            actual: expected_size.saturating_add(1),
        });
    }

    let total_packed = pclusters.len().saturating_mul(PCLUSTER_SIZE);
    if total_packed >= expected_size
        || pclusters
            .iter()
            .any(|pcluster| pcluster.compressed_len > PCLUSTER_SIZE)
    {
        return Ok(None);
    }

    Ok(Some(CompressedLayout {
        pclusters,
        original_size: u64::try_from(expected_size).unwrap_or(u64::MAX),
    }))
}

/// Re-compress one recorded pcluster in a single shot (pass 2 emit).
///
/// # Errors
///
/// Returns an error when buffers are undersized, the stream ends early, or zstd
/// fails.
pub(super) fn recompress_pcluster(
    level: i32,
    reader: &mut dyn Read,
    input_len: usize,
    src: &mut [u8],
    dst: &mut [u8],
) -> Result<usize> {
    if input_len > src.len() || dst.len() < DST_LEN {
        return Err(ErofsError::Internal(
            "pcluster re-emission buffers too small",
        ));
    }
    let Some(input) = src.get_mut(..input_len) else {
        return Err(ErofsError::Internal(
            "pcluster re-emission window out of bounds",
        ));
    };
    reader.read_exact(input).map_err(ErofsError::Io)?;
    let mut cctx = new_cctx(level)?;
    cctx.compress2(dst, input).map_err(compression_error)
}

pub(crate) fn has_representable_compact_indexes(cf: &CompressedLayout) -> bool {
    let block_size = PCLUSTER_SIZE;
    let mut cluster_offset = 0_usize;
    let mut index_count = 0_usize;

    for pcluster in &cf.pclusters {
        let logical_bytes = cluster_offset.saturating_add(pcluster.input_len);

        if logical_bytes < block_size {
            return false;
        }

        if pcluster.input_len < block_size && cf.pclusters.len() > 1 {
            return false;
        }

        index_count = index_count.saturating_add(logical_bytes.div_euclid(block_size));
        cluster_offset = logical_bytes.rem_euclid(block_size);
    }

    if cluster_offset != 0 {
        index_count = index_count.saturating_add(1);
    }

    index_count == usize::try_from(super::lcluster_count(cf)).unwrap_or(usize::MAX)
}

fn top_up(
    reader: &mut dyn Read,
    src: &mut [u8],
    mut buffered: usize,
    want: usize,
    path: &str,
    offset: usize,
    expected_size: usize,
) -> Result<usize> {
    while buffered < want {
        let Some(room) = src.get_mut(buffered..want) else {
            return Err(ErofsError::Internal("chunk fill window out of bounds"));
        };
        let filled = reader.read(room).map_err(|err| {
            ErofsError::Io(std::io::Error::new(err.kind(), format!("{path}: {err}")))
        })?;
        if filled == 0 {
            return Err(ErofsError::FileReadMismatch {
                path: path.to_owned(),
                expected: expected_size,
                actual: offset.saturating_add(buffered),
            });
        }
        buffered = buffered.saturating_add(filled);
    }

    Ok(buffered)
}

fn compress_fitting(
    cctx: &mut CCtx<'_>,
    dst: &mut [u8],
    src: &[u8],
    want: &mut usize,
) -> Result<Option<usize>> {
    loop {
        let Some(prefix) = src.get(..*want) else {
            return Err(ErofsError::Internal("chunk prefix out of bounds"));
        };
        match cctx.compress2(dst, prefix) {
            Ok(written) if written <= PCLUSTER_SIZE => return Ok(Some(written)),
            Ok(_) => {}
            Err(code) if is_destination_too_small(code) => {}
            Err(code) => return Err(compression_error(code)),
        }
        if *want <= PCLUSTER_SIZE {
            return Ok(None);
        }
        *want = (*want >> 1).max(PCLUSTER_SIZE);
    }
}

fn is_destination_too_small(code: usize) -> bool {
    let name = get_error_name(code);

    name.contains("too small") || name.contains("dstSize") || name.contains("Destination buffer")
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use zstd::bulk::decompress;

    use super::super::zstd::PCLUSTER_SIZE;
    use super::{CHUNK_MAX, CHUNK_START, compress_file, has_representable_compact_indexes};
    use crate::compress::model::{CompressedLayout, lcluster_count};
    use crate::error::ErofsError;

    fn xorshift32_bytes(seed: u32, len: usize) -> Vec<u8> {
        let mut state = seed;
        let mut out = Vec::with_capacity(len);
        while out.len() < len {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            out.extend_from_slice(&state.to_le_bytes());
        }
        out.truncate(len);
        out
    }

    fn measure(data: &[u8], level: i32) -> Result<Option<CompressedLayout>, ErofsError> {
        let mut cursor = Cursor::new(data);
        compress_file(&mut cursor, data.len(), "/test", level)
    }

    #[test]
    fn compress_empty_returns_none() {
        // ARRANGE
        let data = b"";

        // ACT
        let result = measure(data, 3).expect("measure");

        // ASSERT
        assert!(result.is_none());
    }

    #[test]
    fn compress_incompressible_returns_none() {
        // ARRANGE
        let data = xorshift32_bytes(0xDEAD_BEEF, 4096);

        // ACT
        let result = measure(&data, 3).expect("measure");

        // ASSERT
        assert!(result.is_none());
    }

    #[test]
    fn compress_compressible_returns_some() {
        // ARRANGE
        let data = vec![0_u8; 8192];

        // ACT
        let layout = measure(&data, 3).expect("measure").expect("layout");

        // ASSERT
        assert!(layout.pclusters.len() <= 2);
        assert_eq!(layout.original_size, 8192);
    }

    #[test]
    fn compress_single_block_compressible_rejects_for_no_savings() {
        // ARRANGE
        let data = vec![0_u8; 4096];

        // ACT
        let result = measure(&data, 3).expect("measure");

        // ASSERT
        assert!(result.is_none());
    }

    #[test]
    fn compress_preserves_original_size() {
        // ARRANGE
        let data = vec![0_u8; 5000];

        // ACT
        let layout = measure(&data, 3).expect("measure").expect("layout");

        // ASSERT
        assert_eq!(layout.original_size, 5000);
    }

    #[test]
    fn compress_absorbs_partial_last_block() {
        // ARRANGE
        let data = vec![0_u8; 4100];

        // ACT
        let layout = measure(&data, 3).expect("measure").expect("layout");

        // ASSERT
        assert_eq!(layout.original_size, 4100);
        assert_eq!(lcluster_count(&layout), 2);
        assert!(has_representable_compact_indexes(&layout));
    }

    #[test]
    fn each_pcluster_fits_in_one_block() {
        // ARRANGE
        let data = vec![0_u8; 131_072];

        // ACT
        let layout = measure(&data, 3).expect("measure").expect("layout");

        // ASSERT
        for pcluster in &layout.pclusters {
            assert!(pcluster.compressed_len <= PCLUSTER_SIZE);
        }
    }

    #[test]
    fn all_input_consumed() {
        // ARRANGE
        let data = vec![0_u8; 131_072];

        // ACT
        let layout = measure(&data, 3).expect("measure").expect("layout");

        // ASSERT
        let total_input: usize = layout
            .pclusters
            .iter()
            .map(|pcluster| pcluster.input_len)
            .sum();
        assert_eq!(total_input, data.len());
    }

    #[test]
    fn adaptive_takes_grow_on_high_ratio_input() {
        // ARRANGE
        let data = vec![0_u8; CHUNK_MAX * 2];

        // ACT
        let layout = measure(&data, 3).expect("measure").expect("layout");

        // ASSERT
        assert!(layout.pclusters.len() <= 5, "few packed pclusters expected");
        assert!(
            layout
                .pclusters
                .iter()
                .any(|pcluster| pcluster.input_len > CHUNK_START),
            "takes should grow past the initial chunk"
        );
        assert!(has_representable_compact_indexes(&layout));
    }

    #[test]
    fn sub_block_tail_is_never_alongside_siblings() {
        // ARRANGE
        for size in [
            CHUNK_START + 1,
            CHUNK_START + 4095,
            CHUNK_START * 2 + 100,
            CHUNK_START + 4097,
        ] {
            let data = vec![0_u8; size];

            // ACT
            let layout = measure(&data, 3).expect("measure").expect("layout");

            // ASSERT
            assert!(
                layout.pclusters.len() == 1
                    || layout
                        .pclusters
                        .iter()
                        .all(|pcluster| pcluster.input_len >= PCLUSTER_SIZE)
            );
            assert!(has_representable_compact_indexes(&layout));
        }
    }

    #[test]
    fn mixed_data_produces_multiple_pclusters() {
        // ARRANGE
        let data: Vec<u8> = (0_u32..64)
            .flat_map(|outer| {
                xorshift32_bytes(
                    outer.saturating_mul(u32::from(u8::from(!outer.is_multiple_of(3)))),
                    4096,
                )
            })
            .collect();

        // ACT
        let result = measure(&data, 3).expect("measure");

        // ASSERT
        if let Some(layout) = result {
            assert!(layout.pclusters.len() > 1);
            let total_input: usize = layout
                .pclusters
                .iter()
                .map(|pcluster| pcluster.input_len)
                .sum();
            assert_eq!(total_input, data.len());
        }
    }

    #[test]
    fn pclusters_recompress_to_recorded_lengths() {
        // ARRANGE
        let data = xorshift32_bytes(0x00C0_FFEE, 3 * CHUNK_START + 1234);
        let Some(layout) = measure(&data, 3).expect("measure") else {
            return;
        };
        let mut offset = 0_usize;

        // ACT & ASSERT
        for pcluster in &layout.pclusters {
            let expected = data
                .get(offset..offset.saturating_add(pcluster.input_len))
                .expect("pcluster input range");
            let mut cursor = Cursor::new(expected);
            let mut src = vec![0_u8; CHUNK_MAX];
            let mut dst = vec![0_u8; PCLUSTER_SIZE + 32];
            let written =
                super::recompress_pcluster(3, &mut cursor, pcluster.input_len, &mut src, &mut dst)
                    .expect("recompress");
            assert_eq!(written, pcluster.compressed_len);
            let compressed = dst.get(..written).expect("compressed bytes");
            let round_tripped = decompress(compressed, pcluster.input_len).expect("decompress");
            assert_eq!(round_tripped, expected);
            offset += pcluster.input_len;
        }
        assert_eq!(offset, data.len());
    }

    #[test]
    fn short_reader_reports_mismatch() {
        // ARRANGE
        let mut cursor = Cursor::new(vec![0_u8; 50]);

        // ACT
        let result = compress_file(&mut cursor, 100, "/short", 3);

        // ASSERT
        assert!(matches!(
            result,
            Err(ErofsError::FileReadMismatch {
                expected: 100,
                actual: 50,
                ..
            })
        ));
    }

    #[test]
    fn overlong_reader_reports_mismatch() {
        // ARRANGE
        let mut cursor = Cursor::new(vec![0_u8; 20]);

        // ACT
        let result = compress_file(&mut cursor, 10, "/long", 3);

        // ASSERT
        assert!(matches!(
            result,
            Err(ErofsError::FileReadMismatch {
                expected: 10,
                actual: 11,
                ..
            })
        ));
    }

    #[test]
    fn measurement_is_deterministic() {
        // ARRANGE
        let data = xorshift32_bytes(0x1234_5678, 70_000);

        // ACT
        let first = measure(&data, 3).expect("measure");
        let second = measure(&data, 3).expect("measure");

        // ASSERT
        assert_eq!(first, second);
    }
}
