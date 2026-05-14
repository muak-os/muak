//! Destsize-bounded zstd compression for EROFS, producing per-pcluster streams.

use crate::BLOCK_SIZE;
use crate::error::{ErofsError, Result};

const ZSTD_DEFAULT_LEVEL: i32 = 3;
const ZSTD_WINDOW_LOG: u32 = 15;
const PCLUSTER_SIZE: usize = BLOCK_SIZE as usize;

/// One independently-decompressible pcluster.
#[derive(Debug, Clone)]
pub struct Pcluster {
    pub compressed_data: Vec<u8>,
    pub input_len: usize,
}

/// Result of compressing a file's data into multiple pclusters.
#[derive(Debug, Clone)]
pub struct CompressedFile {
    pub pclusters: Vec<Pcluster>,
    pub original_size: u64,
}

/// Total number of physical 4KB blocks needed across all pclusters.
pub fn pcluster_blocks(cf: &CompressedFile) -> u32 {
    cf.pclusters.len() as u32
}

/// Number of logical clusters (input 4KB blocks).
pub fn lcluster_count(cf: &CompressedFile) -> u32 {
    let bs = BLOCK_SIZE as usize;
    (cf.original_size as usize).div_ceil(bs) as u32
}

/// Returns whether compact indexes can represent this compressed layout.
pub(crate) fn has_representable_compact_indexes(cf: &CompressedFile) -> bool {
    let bs = BLOCK_SIZE as usize;
    let mut clusterofs = 0usize;
    let mut index_count = 0usize;

    for pc in &cf.pclusters {
        let logical_bytes = clusterofs + pc.input_len;

        if logical_bytes < bs {
            return false;
        }

        index_count += logical_bytes / bs;
        clusterofs = logical_bytes % bs;
    }

    if clusterofs != 0 {
        index_count += 1;
    }

    index_count == lcluster_count(cf) as usize
}

/// Compress file data into multiple destsize-bounded pclusters.
pub fn compress_file(data: &[u8]) -> Result<Option<CompressedFile>> {
    if data.is_empty() {
        return Ok(None);
    }

    let pclusters = destsize_compress_all(data)?;

    let total_compressed: usize = pclusters.len() * PCLUSTER_SIZE;
    if total_compressed >= data.len() {
        return Ok(None);
    }

    Ok(Some(CompressedFile {
        pclusters,
        original_size: data.len() as u64,
    }))
}

fn new_cctx() -> Result<zstd::zstd_safe::CCtx<'static>> {
    let to_err = |e: usize| ErofsError::Compression {
        detail: format!("zstd error code: {e}"),
    };
    let mut cctx = zstd::zstd_safe::CCtx::create();
    cctx.set_parameter(zstd::zstd_safe::CParameter::CompressionLevel(
        ZSTD_DEFAULT_LEVEL,
    ))
    .map_err(to_err)?;
    cctx.set_parameter(zstd::zstd_safe::CParameter::WindowLog(ZSTD_WINDOW_LOG))
        .map_err(to_err)?;
    Ok(cctx)
}

/// Compress all data into multiple pclusters using destsize binary search.
fn destsize_compress_all(data: &[u8]) -> Result<Vec<Pcluster>> {
    let mut cctx = new_cctx()?;
    let mut pclusters = Vec::new();
    let mut offset = 0usize;

    while offset < data.len() {
        let remaining = &data[offset..];
        let (compressed, consumed) = destsize_compress_one(&mut cctx, remaining)?;
        pclusters.push(Pcluster {
            compressed_data: compressed,
            input_len: consumed,
        });
        offset += consumed;
    }
    Ok(pclusters)
}

enum FitblkStep {
    Advance(usize),
    Shrink,
    DoneOk,
    DoneShrink,
}

struct FitblkState {
    l: usize,
    r: usize,
    dstsize: usize,
    l_data: Vec<u8>,
}

fn fitblk_step(
    cctx: &mut zstd::zstd_safe::CCtx<'_>,
    buf: &mut [u8],
    src: &[u8],
    m: usize,
    st: &mut FitblkState,
) -> Result<FitblkStep> {
    let to_err = |e: usize| ErofsError::Compression {
        detail: format!("zstd error code: {e}"),
    };
    match cctx.compress2(buf, &src[..m]) {
        Ok(csize) if csize > 0 && csize <= st.dstsize => {
            st.l_data.clear();
            st.l_data.extend_from_slice(&buf[..csize]);
            if st.r <= m + 1 || csize + 1 >= st.dstsize {
                Ok(FitblkStep::DoneOk)
            } else {
                Ok(FitblkStep::Advance((st.dstsize * m) / csize))
            }
        }
        Ok(_) => {
            if st.r <= st.l + 1 {
                Ok(FitblkStep::DoneShrink)
            } else {
                Ok(FitblkStep::Shrink)
            }
        }
        Err(code) => {
            let err_name = zstd::zstd_safe::get_error_name(code);
            if !err_name.contains("too small")
                && !err_name.contains("dstSize")
                && !err_name.contains("Destination buffer")
            {
                return Err(to_err(code));
            }
            if st.r <= st.l + 1 {
                Ok(FitblkStep::DoneShrink)
            } else {
                Ok(FitblkStep::Shrink)
            }
        }
    }
}

/// Compress one pcluster: find the largest input prefix that fits in PCLUSTER_SIZE.
fn destsize_compress_one(
    cctx: &mut zstd::zstd_safe::CCtx<'_>,
    src: &[u8],
) -> Result<(Vec<u8>, usize)> {
    let dstsize = PCLUSTER_SIZE;
    let buf_size = dstsize + 32;
    let mut fitblk_buffer = vec![0u8; buf_size];
    let mut st = FitblkState {
        l: 0,
        r: src.len() + 1,
        dstsize,
        l_data: Vec::new(),
    };
    let mut m: usize = dstsize * 4;

    loop {
        m = m.max(st.l + 1).min(st.r - 1);
        match fitblk_step(cctx, &mut fitblk_buffer, src, m, &mut st)? {
            FitblkStep::Advance(next_m) => {
                st.l = m;
                m = next_m;
            }
            FitblkStep::Shrink => {
                st.r = m;
                m = (st.l + st.r) / 2;
            }
            FitblkStep::DoneOk => {
                st.l = m;
                break;
            }
            FitblkStep::DoneShrink => break,
        }
    }

    if st.l == 0 {
        let upper = zstd::zstd_safe::compress_bound(src.len());
        let mut dst = vec![0u8; upper];
        let n = cctx
            .compress2(&mut dst, src)
            .map_err(|e| ErofsError::Compression {
                detail: format!("zstd error code: {e}"),
            })?;
        dst.truncate(n);
        return Ok((dst, src.len()));
    }

    Ok((st.l_data, st.l))
}

#[cfg(test)]
mod tests {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    use super::*;

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
        // ARRANGE & ACT
        let result = compress_file(b"").expect("compress_file");

        // ASSERT
        assert!(result.is_none());
    }

    #[test]
    fn compress_incompressible_returns_none() {
        // ARRANGE
        let data = xorshift32_bytes(0xDEAD_BEEF, 4096);

        // ACT
        let result = compress_file(&data).expect("compress_file");

        // ASSERT
        assert!(result.is_none());
    }

    #[test]
    fn compress_compressible_returns_some() {
        // ARRANGE
        let data = vec![0u8; 8192];

        // ACT
        let cf = compress_file(&data).expect("compress_file").expect("cf");

        // ASSERT
        assert!(cf.pclusters.len() <= 2);
        assert_eq!(cf.original_size, 8192);
    }

    #[test]
    fn compress_single_block_compressible() {
        // ARRANGE
        let data = vec![0u8; 4096];

        // ACT
        let result = compress_file(&data).expect("compress_file");

        // ASSERT
        assert!(
            result.is_none(),
            "single block cannot save space: 1 pcluster = 1 block on disk"
        );
    }

    #[test]
    fn compress_preserves_original_size() {
        // ARRANGE
        let data = vec![0u8; 5000];

        // ACT
        let cf = compress_file(&data).expect("compress_file").expect("cf");

        // ASSERT
        assert_eq!(cf.original_size, 5000);
    }

    #[test]
    fn pcluster_blocks_matches_count() {
        // ARRANGE
        let data = vec![0u8; 8192];

        // ACT
        let cf = compress_file(&data).expect("compress_file").expect("cf");

        // ASSERT
        assert_eq!(pcluster_blocks(&cf), cf.pclusters.len() as u32);
    }

    #[test]
    fn lcluster_count_matches_input() {
        // ARRANGE
        let data = vec![0u8; 8192];

        // ACT
        let cf = compress_file(&data).expect("compress_file").expect("cf");

        // ASSERT
        assert_eq!(lcluster_count(&cf), 2);
    }

    #[test]
    fn compress_partial_last_block() {
        // ARRANGE
        let data = vec![0u8; 4100];

        // ACT
        let cf = compress_file(&data).expect("compress_file").expect("cf");

        // ASSERT
        assert_eq!(cf.original_size, 4100);
        assert_eq!(lcluster_count(&cf), 2);
    }

    #[test]
    fn each_pcluster_fits_in_one_block() {
        // ARRANGE
        let data = vec![0u8; 131_072];

        // ACT
        let cf = compress_file(&data).expect("compress_file").expect("cf");

        // ASSERT
        for pc in &cf.pclusters {
            assert!(
                pc.compressed_data.len() <= PCLUSTER_SIZE,
                "pcluster {} > {}",
                pc.compressed_data.len(),
                PCLUSTER_SIZE,
            );
        }
    }

    #[test]
    fn all_input_consumed() {
        // ARRANGE
        let data = vec![0u8; 131_072];

        // ACT
        let cf = compress_file(&data).expect("compress_file").expect("cf");

        // ASSERT
        let total_input: usize = cf.pclusters.iter().map(|p| p.input_len).sum();
        assert_eq!(total_input, data.len());
    }

    #[test]
    fn representable_compact_indexes_accept_cross_block_pclusters() {
        // ARRANGE
        let cf = CompressedFile {
            pclusters: vec![
                Pcluster {
                    compressed_data: vec![0u8; 64],
                    input_len: 5000,
                },
                Pcluster {
                    compressed_data: vec![0u8; 64],
                    input_len: 12_000,
                },
            ],
            original_size: 17_000,
        };

        // ACT & ASSERT
        assert!(has_representable_compact_indexes(&cf));
    }

    #[test]
    fn representable_compact_indexes_reject_tail_end_pclusters() {
        // ARRANGE
        let cf = CompressedFile {
            pclusters: vec![
                Pcluster {
                    compressed_data: vec![0u8; 64],
                    input_len: 5000,
                },
                Pcluster {
                    compressed_data: vec![0u8; 64],
                    input_len: 500,
                },
            ],
            original_size: 5500,
        };

        // ACT & ASSERT
        assert!(!has_representable_compact_indexes(&cf));
    }

    #[test]
    fn each_pcluster_decompresses_correctly() {
        // ARRANGE
        let data = vec![0u8; 131_072];

        // ACT
        let cf = compress_file(&data).expect("compress_file").expect("cf");

        // ASSERT
        let mut offset = 0;
        for pc in &cf.pclusters {
            let expected = &data[offset..offset + pc.input_len];
            let decompressed =
                zstd::bulk::decompress(&pc.compressed_data, pc.input_len).expect("decompress");
            assert_eq!(decompressed, expected);
            offset += pc.input_len;
        }
    }

    #[test]
    fn mixed_data_produces_multiple_pclusters() {
        // ARRANGE
        let mut data = Vec::new();
        for i in 0u64..64 {
            if i % 3 == 0 {
                data.extend_from_slice(&[0u8; 4096]);
            } else {
                let mut chunk = Vec::with_capacity(4096);
                for j in 0u64..256 {
                    let mut h = DefaultHasher::new();
                    (i, j).hash(&mut h);
                    chunk.extend_from_slice(&h.finish().to_le_bytes());
                    chunk.extend_from_slice(&h.finish().to_le_bytes());
                }
                chunk.truncate(4096);
                data.extend_from_slice(&chunk);
            }
        }

        // ACT
        let cf = compress_file(&data).expect("compress_file").expect("cf");

        // ASSERT
        assert!(cf.pclusters.len() > 1, "should produce multiple pclusters");
        let total_input: usize = cf.pclusters.iter().map(|p| p.input_len).sum();
        assert_eq!(total_input, data.len());
    }

    #[test]
    fn debug_compat_pcluster_boundaries() {
        // ARRANGE
        let mut data = Vec::with_capacity(256 * 1024);
        for i in 0u64..64 {
            if i % 3 == 0 {
                data.extend_from_slice(&[0u8; 4096]);
            } else {
                let mut chunk = [0u8; 4096];
                for (j, byte) in chunk.iter_mut().enumerate() {
                    *byte = ((i.wrapping_mul(251).wrapping_add(j as u64).wrapping_mul(167)) & 0xFF)
                        as u8;
                }
                data.extend_from_slice(&chunk);
            }
        }

        // ACT
        eprintln!("Data size: {}", data.len());
        let pclusters = destsize_compress_all(&data).expect("compress");

        // ASSERT
        for (i, pc) in pclusters.iter().enumerate() {
            eprintln!(
                "pcluster[{}]: input_len={} csize={}",
                i,
                pc.input_len,
                pc.compressed_data.len()
            );
        }
        eprintln!("Total pclusters: {}", pclusters.len());
    }
}
