//! Data models and simple counters for compressed EROFS files.

use crate::BLOCK_SIZE;

/// One independently-decompressible pcluster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pcluster {
    /// Number of input bytes covered by this pcluster.
    pub input_len: usize,
    /// Number of bytes the pcluster compresses to (never above `PCLUSTER_SIZE`).
    pub compressed_len: usize,
}

/// Layout-only result of compressing a file's data into multiple pclusters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressedLayout {
    pub pclusters: Vec<Pcluster>,
    pub original_size: u64,
}

/// Total number of physical 4KB blocks needed across all pclusters.
pub fn pcluster_blocks(cf: &CompressedLayout) -> u32 {
    u32::try_from(cf.pclusters.len()).ok().unwrap_or(u32::MAX)
}

/// Number of logical clusters (input 4KB blocks).
pub fn lcluster_count(cf: &CompressedLayout) -> u32 {
    let logical_clusters = cf.original_size.div_ceil(u64::from(BLOCK_SIZE));
    u32::try_from(logical_clusters).ok().unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::{CompressedLayout, Pcluster, lcluster_count, pcluster_blocks};

    #[test]
    fn pcluster_blocks_matches_count() {
        // ARRANGE
        let cf = CompressedLayout {
            pclusters: vec![
                Pcluster {
                    input_len: 4096,
                    compressed_len: 4,
                },
                Pcluster {
                    input_len: 4096,
                    compressed_len: 4,
                },
            ],
            original_size: 8192,
        };

        // ACT & ASSERT
        assert_eq!(pcluster_blocks(&cf), 2);
    }

    #[test]
    fn lcluster_count_matches_input() {
        // ARRANGE
        let cf = CompressedLayout {
            pclusters: vec![Pcluster {
                input_len: 8192,
                compressed_len: 4,
            }],
            original_size: 8192,
        };

        // ACT & ASSERT
        assert_eq!(lcluster_count(&cf), 2);
    }
}
