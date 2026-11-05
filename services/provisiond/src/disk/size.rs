//! Size probing for block devices and regular files.

use std::fs::File;
use std::os::unix::fs::FileTypeExt as _;

use anyhow::Result;

/// Returns the size in bytes of the block device or file behind `file`.
///
/// # Errors
///
/// Returns an error when the metadata lookup fails or when a block device rejects
/// the size ioctl; a failed probe never falls back to a misleading zero size.
pub(crate) fn bytes(file: &File) -> Result<u64> {
    let metadata = file.metadata()?;
    if !metadata.file_type().is_block_device() {
        return Ok(metadata.len());
    }

    Ok(btrfs::format::get_device_size(file)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_returns_byte_length_of_regular_file() {
        // ARRANGE
        let file = tempfile::tempfile().expect("temp file");
        file.set_len(1234).expect("resize");

        // ACT
        let size = bytes(&file).expect("size probe");

        // ASSERT
        assert_eq!(size, 1234, "regular files report their metadata len");
    }
}
