//! Bounded PE header probing of the EFI stub.

use std::io::Read;

use uki::metadata::Metadata;

use crate::error::{Result, YukiError};

/// Upper bound on the PE header prefix the probe will read from the stub.
pub const MAX_HEADER_BYTES: usize = 64 * 1024;

const MIN_HEADER_BYTES: usize = 512;

/// The raw header bytes plus parsed PE metadata.
#[derive(Debug)]
pub struct Probe {
    pub(crate) consumed: u64,
    pub(crate) prefix: Vec<u8>,
    pub(crate) metadata: Metadata,
}

impl Probe {
    /// Bytes consumed from the stub reader (== `size_of_headers`).
    /// The same reader, positioned after this point, is passed to
    /// [`crate::write::write`].
    #[must_use]
    pub fn consumed(&self) -> u64 {
        self.consumed
    }
}

/// Reads and parses the bounded PE header prefix off the stub stream.
///
/// The returned [`Probe`] reports how many bytes were consumed; the caller
/// passes the same stream to [`crate::write::write`], which continues from
/// exactly that position.
///
/// # Errors
///
/// Returns an error when the stub is too short, declares a `size_of_headers`
/// outside the supported range, or is not a valid PE image.
pub fn probe(stub: &mut dyn Read) -> Result<Probe> {
    let mut buf = vec![0_u8; MIN_HEADER_BYTES];
    stub.read_exact(&mut buf)?;

    let size_of_headers = u64::from(uki::metadata::peek_size_of_headers(&buf)?);
    let header_bytes = usize::try_from(size_of_headers)
        .map_err(|_source| YukiError::InvalidPeStructure("size of headers overflow".to_owned()))?;
    if !(MIN_HEADER_BYTES..=MAX_HEADER_BYTES).contains(&header_bytes) {
        return Err(YukiError::InvalidPeStructure(format!(
            "size of headers {header_bytes} outside [{MIN_HEADER_BYTES}, {MAX_HEADER_BYTES}]"
        )));
    }

    buf.resize(header_bytes, 0);
    let tail = buf
        .get_mut(MIN_HEADER_BYTES..header_bytes)
        .ok_or_else(|| YukiError::InvalidPeStructure("headers buffer bounds".to_owned()))?;
    stub.read_exact(tail)?;

    let metadata = uki::metadata::parse(&buf)?;

    Ok(Probe {
        consumed: size_of_headers,
        prefix: buf,
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn write_u32(buf: &mut [u8], offset: usize, value: u32) {
        let end = offset.checked_add(4).unwrap();
        buf.get_mut(offset..end)
            .unwrap()
            .copy_from_slice(&value.to_le_bytes());
    }

    fn write_u16(buf: &mut [u8], offset: usize, value: u16) {
        let end = offset.checked_add(2).unwrap();
        buf.get_mut(offset..end)
            .unwrap()
            .copy_from_slice(&value.to_le_bytes());
    }

    fn minimal_stub() -> Vec<u8> {
        let opt_start = 88_usize;
        let section_start = opt_start.saturating_add(240);
        let headers_raw = section_start.saturating_add(5_usize.saturating_mul(40));
        let headers_aligned = headers_raw.next_multiple_of(512);
        let total_size = headers_aligned.saturating_add(512);

        let mut stub = vec![0_u8; total_size];
        stub.get_mut(0..2).unwrap().copy_from_slice(b"MZ");
        write_u32(&mut stub, 0x3C, 64);
        stub.get_mut(64..68).unwrap().copy_from_slice(b"PE\0\0");
        write_u16(&mut stub, 68, 0x8664);
        write_u16(&mut stub, 70, 1);
        write_u16(&mut stub, 84, 240);
        write_u16(&mut stub, 86, 0x0222);
        write_u16(&mut stub, opt_start, 0x020B);
        write_u32(&mut stub, opt_start.saturating_add(32), 4096);
        write_u32(&mut stub, opt_start.saturating_add(36), 512);
        write_u32(
            &mut stub,
            opt_start.saturating_add(60),
            u32::try_from(headers_aligned).unwrap(),
        );
        write_u16(&mut stub, opt_start.saturating_add(68), 10);
        stub.get_mut(section_start..section_start.saturating_add(5))
            .unwrap()
            .copy_from_slice(b".text");
        write_u32(&mut stub, section_start.saturating_add(8), 512);
        write_u32(&mut stub, section_start.saturating_add(12), 4096);
        write_u32(&mut stub, section_start.saturating_add(16), 512);
        write_u32(
            &mut stub,
            section_start.saturating_add(20),
            u32::try_from(headers_aligned).unwrap(),
        );
        write_u32(&mut stub, section_start.saturating_add(36), 0x6000_0020);

        stub
    }

    #[test]
    fn probe_consumes_exactly_size_of_headers() {
        // ARRANGE
        let stub = minimal_stub();
        let size_of_headers = uki::metadata::peek_size_of_headers(&stub).unwrap();

        // ACT
        let probe = probe(&mut Cursor::new(&stub)).unwrap();

        // ASSERT
        assert_eq!(probe.consumed(), u64::from(size_of_headers));
        assert_eq!(
            probe.prefix.len(),
            usize::try_from(size_of_headers).unwrap()
        );
        assert_eq!(probe.consumed, u64::try_from(probe.prefix.len()).unwrap());
    }

    #[test]
    fn probe_rejects_short_stub() {
        // ARRANGE
        let stub = [0_u8; 511];

        // ACT
        let result = probe(&mut Cursor::new(&stub));

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn probe_rejects_size_of_headers_below_minimum() {
        // ARRANGE
        let mut stub = minimal_stub();
        write_u32(&mut stub, 148, 256);

        // ACT
        let result = probe(&mut Cursor::new(&stub));

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(msg))
                if msg.contains("size of headers")
        ));
    }

    #[test]
    fn probe_rejects_size_of_headers_above_maximum() {
        // ARRANGE
        let mut stub = minimal_stub();
        write_u32(
            &mut stub,
            148,
            u32::try_from(MAX_HEADER_BYTES).unwrap().saturating_add(1),
        );

        // ACT
        let result = probe(&mut Cursor::new(&stub));

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(msg))
                if msg.contains("size of headers")
        ));
    }

    #[test]
    fn probe_rejects_invalid_pe() {
        // ARRANGE
        let stub = vec![0xAB_u8; 512];

        // ACT
        let result = probe(&mut Cursor::new(&stub));

        // ASSERT
        result.unwrap_err();
    }
}
