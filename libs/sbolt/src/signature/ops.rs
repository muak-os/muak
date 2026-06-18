//! Signature utilities: `WIN_CERTIFICATE` builder, hashing, arithmetic.

use ring::digest::Context;

use crate::error::{Result, SboltError};

pub(super) const CERT_TABLE_ENTRY_SIZE: usize = 4;
pub(super) const WIN_CERT_HEADER_SIZE: usize = 8;
const WIN_CERT_REVISION_2_0: u16 = 0x0200;
const WIN_CERT_TYPE_PKCS_SIGNED_DATA: u16 = 0x0002;

/// Build `WIN_CERTIFICATE` for standard Authenticode (type 0x0002).
pub(super) fn build_win_certificate(pkcs7_der: &[u8]) -> Result<Vec<u8>> {
    let total_size = checked_add(
        WIN_CERT_HEADER_SIZE,
        pkcs7_der.len(),
        "WIN_CERTIFICATE size",
    )?;
    let total_size_u32 = usize_to_u32(total_size, "WIN_CERTIFICATE")?;
    let mut result = Vec::with_capacity(total_size);
    result.extend_from_slice(&total_size_u32.to_le_bytes());
    result.extend_from_slice(&WIN_CERT_REVISION_2_0.to_le_bytes());
    result.extend_from_slice(&WIN_CERT_TYPE_PKCS_SIGNED_DATA.to_le_bytes());
    result.extend_from_slice(pkcs7_der);

    Ok(result)
}

/// Hash a range of data, excluding specified regions.
pub(super) fn hash_range_excluding(
    ctx: &mut Context,
    data: &[u8],
    start: usize,
    end: usize,
    exclusions: &[(usize, usize)],
) -> Result<()> {
    let mut exclusions: Vec<_> = exclusions
        .iter()
        .filter(|&&(off, _)| off >= start && off < end)
        .copied()
        .collect();
    exclusions.sort_by_key(|&(off, _)| off);

    let mut pos = start;
    for (excl_off, excl_len) in exclusions {
        if pos < excl_off {
            let range_bytes = data
                .get(pos..excl_off)
                .ok_or_else(|| SboltError::PeOperation("excluded range exceeds file".into()))?;
            ctx.update(range_bytes);
        }
        pos = checked_add(excl_off, excl_len, "excluded range end")?;
    }

    if pos < end {
        let range_bytes = data
            .get(pos..end)
            .ok_or_else(|| SboltError::PeOperation("hashed range exceeds file".into()))?;
        ctx.update(range_bytes);
    }

    Ok(())
}

pub(super) fn put_u32_le(data: &mut [u8], offset: usize, value: u32) -> Result<()> {
    let end = checked_add(offset, CERT_TABLE_ENTRY_SIZE, "write_u32 range end")?;
    let bytes = data
        .get_mut(offset..end)
        .ok_or_else(|| SboltError::PeOperation("write u32 beyond buffer".into()))?;
    bytes.copy_from_slice(&value.to_le_bytes());

    Ok(())
}

pub(super) fn usize_to_u32(value: usize, context: &str) -> Result<u32> {
    u32::try_from(value).map_err(|e| SboltError::PeOperation(format!("{context} exceeds u32: {e}")))
}

pub(super) fn u32_to_usize(value: u32, context: &str) -> Result<usize> {
    usize::try_from(value)
        .map_err(|e| SboltError::PeOperation(format!("{context} exceeds usize: {e}")))
}

pub(super) fn checked_add(lhs: usize, rhs: usize, context: &str) -> Result<usize> {
    lhs.checked_add(rhs)
        .ok_or_else(|| SboltError::PeOperation(format!("{context} overflow")))
}

pub(super) fn align_to(value: usize, alignment: usize, context: &str) -> Result<usize> {
    let alignment_mask = alignment
        .checked_sub(1)
        .ok_or_else(|| SboltError::PeOperation(format!("{context} invalid alignment")))?;
    let adjusted = value
        .checked_add(alignment_mask)
        .ok_or_else(|| SboltError::PeOperation(format!("{context} overflow")))?;

    Ok(adjusted & !alignment_mask)
}
