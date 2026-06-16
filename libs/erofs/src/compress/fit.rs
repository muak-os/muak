//! Destsize-fitting compression search and compact-index compatibility checks.

use zstd::zstd_safe::CCtx;

use super::model::{CompressedFile, Pcluster, lcluster_count};
use super::zstd::{PCLUSTER_SIZE, compress_whole_input, compression_error, error_name, new_cctx};
use crate::error::{ErofsError, Result};

pub(super) enum FitblkStep {
    Advance(usize),
    Shrink,
    DoneOk,
    DoneShrink,
}

pub(super) struct FitblkState {
    pub(super) lower_bound: usize,
    pub(super) upper_bound: usize,
    pub(super) dstsize: usize,
    pub(super) best_data: Vec<u8>,
}

pub(crate) fn has_representable_compact_indexes(cf: &CompressedFile) -> bool {
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

    index_count
        == usize::try_from(lcluster_count(cf))
            .ok()
            .unwrap_or(usize::MAX)
}

pub(super) fn compress_file(data: &[u8], compression_level: i32) -> Result<Option<CompressedFile>> {
    if data.is_empty() {
        return Ok(None);
    }

    let pclusters = destsize_compress_all(data, compression_level)?;

    let total_compressed = pclusters.len().saturating_mul(PCLUSTER_SIZE);
    if total_compressed >= data.len() {
        return Ok(None);
    }

    if pclusters
        .iter()
        .any(|pc| pc.compressed_data.len() > PCLUSTER_SIZE)
    {
        return Ok(None);
    }

    Ok(Some(CompressedFile {
        pclusters,
        original_size: u64::try_from(data.len()).ok().unwrap_or(u64::MAX),
    }))
}

pub(super) fn destsize_compress_all(data: &[u8], compression_level: i32) -> Result<Vec<Pcluster>> {
    let mut cctx = new_cctx(compression_level)?;
    let mut pclusters = Vec::new();
    let mut offset = 0_usize;

    while offset < data.len() {
        let remaining = data.get(offset..).unwrap_or_default();
        let (compressed, consumed) = destsize_compress_one(&mut cctx, remaining)?;
        pclusters.push(Pcluster {
            compressed_data: compressed,
            input_len: consumed,
        });
        offset = offset.saturating_add(consumed);
    }
    Ok(pclusters)
}

pub(super) fn fitblk_step(
    cctx: &mut CCtx<'_>,
    buf: &mut [u8],
    src: &[u8],
    probe_len: usize,
    state: &mut FitblkState,
) -> Result<FitblkStep> {
    let Some(src_prefix) = src.get(..probe_len) else {
        return Err(ErofsError::Compression {
            detail: "compression probe exceeded input length".to_owned(),
        });
    };

    match cctx.compress2(buf, src_prefix) {
        Ok(compressed_size) if compressed_size > 0 && compressed_size <= state.dstsize => {
            let compressed_data = buf.get(..compressed_size).unwrap_or_default();
            state.best_data.clear();
            state.best_data.extend_from_slice(compressed_data);
            if state.upper_bound <= probe_len.saturating_add(1)
                || compressed_size.saturating_add(1) >= state.dstsize
            {
                Ok(FitblkStep::DoneOk)
            } else {
                let next_probe = state
                    .dstsize
                    .checked_mul(probe_len)
                    .map_or(usize::MAX, |value| value.div_euclid(compressed_size));
                Ok(FitblkStep::Advance(next_probe))
            }
        }
        Ok(_) => {
            if state.upper_bound <= state.lower_bound.saturating_add(1) {
                Ok(FitblkStep::DoneShrink)
            } else {
                Ok(FitblkStep::Shrink)
            }
        }
        Err(code) => fitblk_step_from_error_name(error_name(code), code, state),
    }
}

pub(super) fn destsize_compress_one(cctx: &mut CCtx<'_>, src: &[u8]) -> Result<(Vec<u8>, usize)> {
    let dstsize = PCLUSTER_SIZE;
    let buffer_size = dstsize.saturating_add(32);
    let mut fitblk_buffer = vec![0_u8; buffer_size];
    let mut state = FitblkState {
        lower_bound: 0,
        upper_bound: src.len().saturating_add(1),
        dstsize,
        best_data: Vec::new(),
    };
    let mut probe_len = dstsize.saturating_mul(4);

    loop {
        let min_probe = state.lower_bound.saturating_add(1);
        let max_probe = state.upper_bound.saturating_sub(1);
        probe_len = probe_len.max(min_probe).min(max_probe);
        match fitblk_step(cctx, &mut fitblk_buffer, src, probe_len, &mut state)? {
            FitblkStep::Advance(next_probe) => {
                state.lower_bound = probe_len;
                probe_len = next_probe;
            }
            FitblkStep::Shrink => {
                state.upper_bound = probe_len;
                probe_len = usize::midpoint(state.lower_bound, state.upper_bound);
            }
            FitblkStep::DoneOk => {
                state.lower_bound = probe_len;
                break;
            }
            FitblkStep::DoneShrink => break,
        }
    }

    if state.lower_bound == 0 {
        let bounded = src.len().min(PCLUSTER_SIZE);
        let bounded_src = src.get(..bounded).unwrap_or(src);

        return Ok((compress_whole_input(cctx, bounded_src)?, bounded_src.len()));
    }

    Ok((state.best_data, state.lower_bound))
}

pub(super) fn fitblk_step_from_error_name(
    err_name: &str,
    code: usize,
    state: &FitblkState,
) -> Result<FitblkStep> {
    if !err_name.contains("too small")
        && !err_name.contains("dstSize")
        && !err_name.contains("Destination buffer")
    {
        return Err(compression_error(code));
    }
    if state.upper_bound <= state.lower_bound.saturating_add(1) {
        Ok(FitblkStep::DoneShrink)
    } else {
        Ok(FitblkStep::Shrink)
    }
}

#[cfg(test)]
mod tests {
    use zstd::bulk::decompress;

    use super::{
        FitblkState, FitblkStep, compress_file, destsize_compress_all, destsize_compress_one,
        fitblk_step, fitblk_step_from_error_name, has_representable_compact_indexes,
    };
    use crate::compress::model::{CompressedFile, Pcluster, lcluster_count};
    use crate::compress::zstd::{PCLUSTER_SIZE, new_cctx};
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

    #[test]
    fn compress_empty_returns_none() {
        // ARRANGE
        let result = compress_file(b"", 3).expect("compress_file");
        // ACT
        // ASSERT
        assert!(result.is_none());
    }

    #[test]
    fn compress_incompressible_returns_none() {
        // ARRANGE
        let data = xorshift32_bytes(0xDEAD_BEEF, 4096);
        let result = compress_file(&data, 3).expect("compress_file");
        // ACT
        // ASSERT
        assert!(result.is_none());
    }

    #[test]
    fn compress_compressible_returns_some() {
        // ARRANGE
        let data = vec![0_u8; 8192];
        let cf = compress_file(&data, 3).expect("compress_file").expect("cf");
        // ACT
        // ASSERT
        assert!(cf.pclusters.len() <= 2);
        assert_eq!(cf.original_size, 8192);
    }

    #[test]
    fn compress_single_block_compressible() {
        // ARRANGE
        let data = vec![0_u8; 4096];
        let result = compress_file(&data, 3).expect("compress_file");
        // ACT
        // ASSERT
        assert!(result.is_none());
    }

    #[test]
    fn compress_preserves_original_size() {
        // ARRANGE
        let data = vec![0_u8; 5000];
        let cf = compress_file(&data, 3).expect("compress_file").expect("cf");
        // ACT
        // ASSERT
        assert_eq!(cf.original_size, 5000);
    }

    #[test]
    fn compress_partial_last_block() {
        // ARRANGE
        let data = vec![0_u8; 4100];
        let cf = compress_file(&data, 3).expect("compress_file").expect("cf");
        // ACT
        // ASSERT
        assert_eq!(cf.original_size, 4100);
        assert_eq!(lcluster_count(&cf), 2);
    }

    #[test]
    fn each_pcluster_fits_in_one_block() {
        // ARRANGE
        let data = vec![0_u8; 131_072];
        let cf = compress_file(&data, 3).expect("compress_file").expect("cf");
        for pcluster in &cf.pclusters {
            // ACT
            // ASSERT
            assert!(pcluster.compressed_data.len() <= PCLUSTER_SIZE);
        }
    }

    #[test]
    fn all_input_consumed() {
        // ARRANGE
        let data = vec![0_u8; 131_072];
        let cf = compress_file(&data, 3).expect("compress_file").expect("cf");
        let total_input: usize = cf.pclusters.iter().map(|pcluster| pcluster.input_len).sum();
        // ACT
        // ASSERT
        assert_eq!(total_input, data.len());
    }

    #[test]
    fn representable_compact_indexes_accept_cross_block_pclusters() {
        // ARRANGE
        let cf = CompressedFile {
            pclusters: vec![
                Pcluster {
                    compressed_data: vec![0_u8; 64],
                    input_len: 5000,
                },
                Pcluster {
                    compressed_data: vec![0_u8; 64],
                    input_len: 12_000,
                },
            ],
            original_size: 17_000,
        };
        // ACT
        // ASSERT
        assert!(has_representable_compact_indexes(&cf));
    }

    #[test]
    fn representable_compact_indexes_reject_tail_end_pclusters() {
        // ARRANGE
        let cf = CompressedFile {
            pclusters: vec![
                Pcluster {
                    compressed_data: vec![0_u8; 64],
                    input_len: 5000,
                },
                Pcluster {
                    compressed_data: vec![0_u8; 64],
                    input_len: 500,
                },
            ],
            original_size: 5500,
        };
        // ACT
        // ASSERT
        assert!(!has_representable_compact_indexes(&cf));
    }

    #[test]
    fn each_pcluster_decompresses_correctly() {
        // ARRANGE
        let data = vec![0_u8; 131_072];
        let cf = compress_file(&data, 3).expect("compress_file").expect("cf");
        let mut offset = 0;
        for pcluster in &cf.pclusters {
            let expected = data
                .get(offset..offset + pcluster.input_len)
                .expect("pcluster input range");
            let decompressed =
                decompress(&pcluster.compressed_data, pcluster.input_len).expect("decompress");
            // ACT
            // ASSERT
            assert_eq!(decompressed, expected);
            offset += pcluster.input_len;
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

        let result = compress_file(&data, 3).expect("compress_file");
        // ACT
        // ASSERT
        if let Some(cf) = result {
            assert!(cf.pclusters.len() > 1);
            let total_input: usize = cf.pclusters.iter().map(|pcluster| pcluster.input_len).sum();
            assert_eq!(total_input, data.len());
        }
    }

    #[test]
    fn debug_compat_pcluster_boundaries() {
        // ARRANGE
        let data: Vec<u8> = (0_u32..64)
            .flat_map(|outer| {
                xorshift32_bytes(
                    outer.saturating_mul(u32::from(u8::from(!outer.is_multiple_of(3)))),
                    4096,
                )
            })
            .collect();

        let pclusters = destsize_compress_all(&data, 3).expect("compress");
        // ACT
        // ASSERT
        assert!(!pclusters.is_empty());
    }

    #[test]
    fn fitblk_step_reports_probe_beyond_input() {
        // ARRANGE
        let mut cctx = new_cctx(3).expect("cctx");
        let mut buf = vec![0_u8; PCLUSTER_SIZE.saturating_add(32)];
        let mut state = FitblkState {
            lower_bound: 0,
            upper_bound: 8,
            dstsize: PCLUSTER_SIZE,
            best_data: Vec::new(),
        };

        let result = fitblk_step(&mut cctx, &mut buf, b"abc", 4, &mut state);

        // ACT
        // ASSERT
        assert!(matches!(
            result,
            Err(ErofsError::Compression { detail })
                if detail.contains("probe exceeded input length")
        ));
    }

    #[test]
    fn destsize_compress_one_falls_back_when_nothing_fits() {
        // ARRANGE
        let mut cctx = new_cctx(3).expect("cctx");
        let data = xorshift32_bytes(0x1234_5678, PCLUSTER_SIZE.saturating_mul(2));

        let (compressed, consumed) = destsize_compress_one(&mut cctx, &data).expect("compress one");

        // ACT
        // ASSERT
        assert!(consumed > 0);
        assert!(consumed <= data.len());
        assert!(!compressed.is_empty());
        assert!(
            compressed.len() <= PCLUSTER_SIZE,
            "fallback pcluster compressed data {} exceeds PCLUSTER_SIZE {}",
            compressed.len(),
            PCLUSTER_SIZE,
        );
    }

    #[test]
    fn incompressible_pclusters_fit_in_one_block() {
        for size in [4097, 8192, 10000, 20000, 65536] {
            // ARRANGE
            let random = xorshift32_bytes(0xDEAD, size);
            if let Ok(Some(cf)) = compress_file(&random, 3) {
                // ACT
                for (i, p) in cf.pclusters.iter().enumerate() {
                    // ASSERT
                    assert!(
                        p.compressed_data.len() <= PCLUSTER_SIZE,
                        "size={}: pcluster[{i}] compressed {} > {}",
                        size,
                        p.compressed_data.len(),
                        PCLUSTER_SIZE,
                    );
                }
            }
        }
    }

    #[test]
    fn mixed_pclusters_fit_in_one_block() {
        for size in [4097, 8192, 10000, 20000, 65536] {
            // ARRANGE
            let mut mixed = Vec::with_capacity(size);
            let half = size / 2;
            mixed.extend_from_slice(&xorshift32_bytes(0xBEEF, half));
            mixed.extend(std::iter::repeat(0x00).take(size - half));
            if let Ok(Some(cf)) = compress_file(&mixed, 3) {
                // ACT
                for (i, p) in cf.pclusters.iter().enumerate() {
                    // ASSERT
                    assert!(
                        p.compressed_data.len() <= PCLUSTER_SIZE,
                        "size={}: pcluster[{i}] compressed {} > {}",
                        size,
                        p.compressed_data.len(),
                        PCLUSTER_SIZE,
                    );
                }
            }
        }
    }

    #[test]
    fn fitblk_step_from_error_name_distinguishes_shrinkable_and_fatal_errors() {
        // ARRANGE
        let shrink_state = FitblkState {
            lower_bound: 1,
            upper_bound: 4,
            dstsize: PCLUSTER_SIZE,
            best_data: Vec::new(),
        };
        let done_state = FitblkState {
            lower_bound: 1,
            upper_bound: 2,
            dstsize: PCLUSTER_SIZE,
            best_data: Vec::new(),
        };

        let shrink = fitblk_step_from_error_name("dstSize too small", 1, &shrink_state);
        let done = fitblk_step_from_error_name("Destination buffer too small", 1, &done_state);
        let fatal = fitblk_step_from_error_name("fatal", 7, &shrink_state);

        // ACT
        // ASSERT
        assert!(matches!(shrink, Ok(FitblkStep::Shrink)));
        assert!(matches!(done, Ok(FitblkStep::DoneShrink)));
        assert!(matches!(fatal, Err(ErofsError::Compression { detail }) if detail.contains('7')));
    }
}
