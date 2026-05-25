//! Shared offset and size helpers for writer submodules.

use crate::error::{ErofsError, Result};
use crate::{BLOCK_SIZE, SLOT_SIZE};

pub(super) fn mul(lhs: usize, rhs: usize) -> Option<usize> {
    lhs.checked_mul(rhs)
}

pub(super) fn full_block_bytes(data_blocks: u32, block_size: usize) -> Result<usize> {
    mul(usize_from_u32(data_blocks), block_size)
        .ok_or(ErofsError::Internal("inline block byte count overflow"))
}

pub(super) fn block_offset(
    data_blkaddr: u32,
    block_size: usize,
    context: &'static str,
) -> Result<usize> {
    let base = usize_from_u32(data_blkaddr);

    mul(base, block_size).ok_or(match context {
        "inline data" => ErofsError::Internal("inline data offset overflow"),
        "plain data" => ErofsError::Internal("plain data offset overflow"),
        _ => ErofsError::Internal("compressed data offset overflow"),
    })
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
    use super::{block_offset, full_block_bytes};
    use crate::error::ErofsError;

    #[test]
    fn helper_offsets_report_expected_errors() {
        // ARRANGE
        let full_block_error = full_block_bytes(u32::MAX, usize::MAX);
        let inline_offset_error = block_offset(u32::MAX, usize::MAX, "inline data");
        let plain_offset_error = block_offset(u32::MAX, usize::MAX, "plain data");
        let compressed_offset_error = block_offset(u32::MAX, usize::MAX, "compressed data");

        // ACT
        // ASSERT
        assert!(matches!(
            full_block_error,
            Err(ErofsError::Internal("inline block byte count overflow"))
        ));
        assert!(matches!(
            inline_offset_error,
            Err(ErofsError::Internal("inline data offset overflow"))
        ));
        assert!(matches!(
            plain_offset_error,
            Err(ErofsError::Internal("plain data offset overflow"))
        ));
        assert!(matches!(
            compressed_offset_error,
            Err(ErofsError::Internal("compressed data offset overflow"))
        ));
    }
}
