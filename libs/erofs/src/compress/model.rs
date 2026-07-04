//! Data models and simple counters for compressed EROFS files.

use crate::BLOCK_SIZE;

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
    u32::try_from(cf.pclusters.len()).ok().unwrap_or(u32::MAX)
}

/// Number of logical clusters (input 4KB blocks).
pub fn lcluster_count(cf: &CompressedFile) -> u32 {
    let logical_clusters = cf.original_size.div_ceil(u64::from(BLOCK_SIZE));
    u32::try_from(logical_clusters).ok().unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::{CompressedFile, Pcluster, lcluster_count, pcluster_blocks};

    #[test]
    fn pcluster_blocks_matches_count() {
        // ARRANGE
        let cf = CompressedFile {
            pclusters: vec![
                Pcluster {
                    compressed_data: vec![0_u8; 4],
                    input_len: 4096,
                },
                Pcluster {
                    compressed_data: vec![0_u8; 4],
                    input_len: 4096,
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
        let cf = CompressedFile {
            pclusters: vec![Pcluster {
                compressed_data: vec![0_u8; 4],
                input_len: 8192,
            }],
            original_size: 8192,
        };

        // ACT & ASSERT
        assert_eq!(lcluster_count(&cf), 2);
    }
}
