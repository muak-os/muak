//! Byte readers, writers, and checksum helpers shared by the parsers and builders.

use core::net::Ipv6Addr;
use core::ops::Range;

use crate::{Failure, Result, checksum};

/// Offset of the `ICMPv6` checksum field.
pub(crate) const CHECKSUM: usize = 2;
/// Length of the `ICMPv6` checksum field.
pub(crate) const CHECKSUM_LEN: usize = 2;
/// Length of an IPv6 address in bytes.
pub(crate) const ADDRESS_LEN: usize = 16;

/// Returns the sub-slice of `data` described by `range`, or `None` when it does not fit.
pub(crate) fn slot(data: &mut [u8], range: Range<usize>) -> Option<&mut [u8]> {
    data.get_mut(range)
}

/// Reads the byte at `offset`, or reports the data as truncated.
pub(crate) fn byte(data: &[u8], offset: usize) -> Result<u8> {
    data.get(offset)
        .copied()
        .ok_or(Failure::Truncated(data.len(), offset.saturating_add(1)))
}

/// Reads the big-endian `u16` at `offset`.
pub(crate) fn u16_at(data: &[u8], offset: usize) -> Result<u16> {
    data.get(offset..)
        .and_then(<[u8]>::first_chunk::<2>)
        .map(|bytes| u16::from_be_bytes(*bytes))
        .ok_or(Failure::Truncated(data.len(), offset.saturating_add(2)))
}

/// Reads the big-endian `u32` at `offset`.
pub(crate) fn u32_at(data: &[u8], offset: usize) -> Result<u32> {
    data.get(offset..)
        .and_then(<[u8]>::first_chunk::<4>)
        .map(|bytes| u32::from_be_bytes(*bytes))
        .ok_or(Failure::Truncated(data.len(), offset.saturating_add(4)))
}

/// Reads the IPv6 address at `offset`.
pub(crate) fn address_at(data: &[u8], offset: usize) -> Result<Ipv6Addr> {
    data.get(offset..)
        .and_then(<[u8]>::first_chunk::<ADDRESS_LEN>)
        .map(|octets| Ipv6Addr::from(*octets))
        .ok_or(Failure::Truncated(
            data.len(),
            offset.saturating_add(ADDRESS_LEN),
        ))
}

/// Copies `bytes` into `buffer` at `cursor` and returns the cursor after them.
pub(crate) fn put(buffer: &mut [u8], cursor: usize, bytes: &[u8]) -> Result<usize> {
    let end = cursor.saturating_add(bytes.len());
    slot(buffer, cursor..end)
        .ok_or(Failure::TooSmall(end))?
        .copy_from_slice(bytes);

    Ok(end)
}

/// Writes the `ICMPv6` checksum of the first `end` bytes of `message` and returns them.
pub(crate) fn seal(
    message: &mut [u8],
    end: usize,
    source: Ipv6Addr,
    destination: Ipv6Addr,
) -> Result<&mut [u8]> {
    let written = message.get_mut(..end).ok_or(Failure::TooSmall(end))?;
    let sum = checksum(source, destination, written);
    let field_end = CHECKSUM.saturating_add(CHECKSUM_LEN);
    slot(written, CHECKSUM..field_end)
        .ok_or(Failure::TooSmall(field_end))?
        .copy_from_slice(&sum.to_be_bytes());

    Ok(written)
}
