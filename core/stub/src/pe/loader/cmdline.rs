//! Command line encoding for PE/COFF loader.

use core::ptr;

use anyhow::{Context as _, Result};
use uefi::boot::{MemoryType, allocate_pool};

use crate::util::strip_trailing_cmdline_terminators;

/// Converts an ASCII command line to a UCS-2 (UTF-16LE) buffer in pool memory.
pub(super) fn encode_ucs2(cmdline: &[u8]) -> Result<(*mut u8, u32)> {
    let cmd = strip_trailing_cmdline_terminators(cmdline);
    if cmd.is_empty() {
        return Ok((ptr::null_mut(), 0));
    }

    let ucs2_len = cmd
        .len()
        .checked_add(1)
        .context("command line length overflow")?;
    let byte_size = ucs2_len
        .checked_mul(size_of::<u16>())
        .context("command line byte length overflow")?;
    let load_options_size = u32::try_from(byte_size).context("command line buffer too large")?;

    let ptr = allocate_pool(MemoryType::LOADER_DATA, byte_size)
        .context("failed to allocate command line buffer")?
        .as_ptr();

    // SAFETY: ptr is freshly allocated for `byte_size` bytes.
    let bytes = unsafe { core::slice::from_raw_parts_mut(ptr, byte_size) };
    for (chunk, byte) in bytes
        .chunks_exact_mut(size_of::<u16>())
        .zip(cmd.iter().copied())
    {
        chunk.copy_from_slice(&[byte, 0]);
    }

    let terminator = bytes
        .get_mut(byte_size - size_of::<u16>()..byte_size)
        .context("command line terminator slice out of bounds")?;
    terminator.copy_from_slice(&0u16.to_le_bytes());

    Ok((ptr, load_options_size))
}

#[cfg(test)]
mod tests {
    #[test]
    fn encode_ucs2_accounts_for_utf16_nul_terminator_size() {
        // ARRANGE
        let cmdline = b"abc";
        let byte_size = (cmdline.len() + 1) * size_of::<u16>();

        // ACT & ASSERT
        assert_eq!(byte_size, 8);
    }
}
