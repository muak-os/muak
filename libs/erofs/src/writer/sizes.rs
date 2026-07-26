//! Size and offset arithmetic helpers for writer submodules.

use crate::error::{ErofsError, Result};
use crate::{BLOCK_SIZE, SLOT_SIZE};

pub(super) fn mul(lhs: usize, rhs: usize) -> Option<usize> {
    lhs.checked_mul(rhs)
}

pub(super) fn full_block_bytes(data_blocks: u32, block_size: usize) -> Result<usize> {
    mul(usize_from_u32(data_blocks), block_size)
        .ok_or(ErofsError::Internal("inline block byte count overflow"))
}

pub(super) fn block_size_usize() -> usize {
    usize_from_u32(BLOCK_SIZE)
}

pub(super) fn usize_from_u32(value: u32) -> usize {
    usize::try_from(value).ok().unwrap_or(usize::MAX)
}

pub(super) fn slot_offset(nid: u64) -> Result<usize> {
    mul(
        usize::try_from(nid)
            .ok()
            .ok_or(ErofsError::Internal("inode nid does not fit usize"))?,
        SLOT_SIZE,
    )
    .ok_or(ErofsError::Internal("inode slot offset overflow"))
}

#[cfg(test)]
mod tests {
    use super::full_block_bytes;
    use crate::error::ErofsError;

    #[test]
    fn helper_offsets_report_expected_errors() {
        // ARRANGE

        // ACT
        let full_block_error = full_block_bytes(u32::MAX, usize::MAX);

        // ASSERT
        assert!(matches!(
            full_block_error,
            Err(ErofsError::Internal("inline block byte count overflow"))
        ));
    }
}
