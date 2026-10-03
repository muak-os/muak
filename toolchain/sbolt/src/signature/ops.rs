//! Signature utilities: `WIN_CERTIFICATE` builder, hashing, arithmetic.

use sha2::{Digest as _, Sha256};

use crate::error::{Result, SboltError};

pub(super) const CERT_TABLE_ENTRY_SIZE: usize = 4;
pub(super) const WIN_CERT_HEADER_SIZE: usize = 8;
const WIN_CERT_REVISION_2_0: u16 = 0x0200;
const WIN_CERT_TYPE_PKCS_SIGNED_DATA: u16 = 0x0002;

/// Build `WIN_CERTIFICATE` for standard Authenticode (type 0x0002).
pub(super) fn build_win_certificate(pkcs7_der: &[u8]) -> Result<Vec<u8>> {
    let total_size = WIN_CERT_HEADER_SIZE
        .checked_add(pkcs7_der.len())
        .ok_or_else(|| SboltError::PeOperation("WIN_CERTIFICATE size overflow".into()))?;
    let total_size_u32 = u32::try_from(total_size)
        .map_err(|e| SboltError::PeOperation(format!("WIN_CERTIFICATE exceeds u32: {e}")))?;
    let mut result = Vec::with_capacity(total_size);
    result.extend_from_slice(&total_size_u32.to_le_bytes());
    result.extend_from_slice(&WIN_CERT_REVISION_2_0.to_le_bytes());
    result.extend_from_slice(&WIN_CERT_TYPE_PKCS_SIGNED_DATA.to_le_bytes());
    result.extend_from_slice(pkcs7_der);

    Ok(result)
}

/// Hash a range of data, excluding specified regions.
pub(super) fn hash_range_excluding(
    ctx: &mut Sha256,
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
        pos = excl_off
            .checked_add(excl_len)
            .ok_or_else(|| SboltError::PeOperation("excluded range end overflow".into()))?;
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
    let end = offset
        .checked_add(CERT_TABLE_ENTRY_SIZE)
        .ok_or_else(|| SboltError::PeOperation("write_u32 range end overflow".into()))?;
    let bytes = data
        .get_mut(offset..end)
        .ok_or_else(|| SboltError::PeOperation("write u32 beyond buffer".into()))?;
    bytes.copy_from_slice(&value.to_le_bytes());

    Ok(())
}
