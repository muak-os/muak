//! Data models and simple counters for compressed EROFS files.

use super::CompressedLayout;
use crate::BLOCK_SIZE;

/// Total number of physical 4KB blocks needed across all pclusters.
pub(super) fn pcluster_blocks(cf: &CompressedLayout) -> u32 {
    u32::try_from(cf.pclusters.len()).ok().unwrap_or(u32::MAX)
}

/// Number of logical clusters (input 4KB blocks).
pub(super) fn lcluster_count(cf: &CompressedLayout) -> u32 {
    let logical_clusters = cf.original_size.div_ceil(u64::from(BLOCK_SIZE));
    u32::try_from(logical_clusters).ok().unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::{lcluster_count, pcluster_blocks};
    use crate::compress::{CompressedLayout, Pcluster};

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
