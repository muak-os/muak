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

    Ok((ptr, load_options_size))
}
