//! TPM2 response parsing helpers.

use crate::error::{Result, Tpm2Error};
use crate::tags::{TPM2_ST_NO_SESSIONS, TPM2_ST_SESSIONS};

pub(crate) const RESPONSE_HEADER_SIZE: usize = 10;

/// Reader for parsing TPM2 response buffers.
pub(crate) struct ResponseReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> ResponseReader<'a> {
    #[must_use = "dropping it discards the parsed response"]
    pub(crate) const fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub(crate) fn read_u8(&mut self) -> Result<u8> {
        let bytes = self.read_array::<1>()?;
        Ok(bytes[0])
    }

    pub(crate) fn read_u16(&mut self) -> Result<u16> {
        let bytes = self.read_array::<2>()?;
        Ok(u16::from_be_bytes(bytes))
    }

    pub(crate) fn read_u32(&mut self) -> Result<u32> {
        let bytes = self.read_array::<4>()?;
        Ok(u32::from_be_bytes(bytes))
    }

    pub(crate) fn read_bytes(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self.checked_end(len)?;
        let slice = self
            .data
            .get(self.pos..end)
            .ok_or(Tpm2Error::ResponseTooShort {
                expected: end,
                actual: self.data.len(),
            })?;
        self.pos = end;
        Ok(slice)
    }

    pub(crate) fn read_sized(&mut self) -> Result<&'a [u8]> {
        let len = usize::from(self.read_u16()?);
        self.read_bytes(len)
    }

    const fn checked_end(&self, len: usize) -> Result<usize> {
        let end = self.pos.saturating_add(len);
        if end > self.data.len() {
            return Err(Tpm2Error::ResponseTooShort {
                expected: end,
                actual: self.data.len(),
            });
        }
        Ok(end)
    }

    fn read_array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let bytes = self.read_bytes(N)?;
        let mut array = [0_u8; N];
        array.copy_from_slice(bytes);
        Ok(array)
    }
}

pub(crate) struct ResponseBody<'a> {
    reader: ResponseReader<'a>,
}

impl<'a> ResponseBody<'a> {
    pub(crate) fn from_response(response: &'a [u8]) -> Result<Self> {
        let Some(body) = response.get(RESPONSE_HEADER_SIZE..) else {
            return Err(Tpm2Error::ResponseTooShort {
                expected: RESPONSE_HEADER_SIZE,
                actual: response.len(),
            });
        };

        Ok(Self {
            reader: ResponseReader::new(body),
        })
    }

    pub(crate) fn read_u8(&mut self) -> Result<u8> {
        self.reader.read_u8()
    }

    pub(crate) fn read_u32(&mut self) -> Result<u32> {
        self.reader.read_u32()
    }

    pub(crate) fn read_handle<H>(&mut self) -> Result<H>
    where
        H: From<u32>,
    {
        self.reader.read_u32().map(H::from)
    }

    pub(crate) fn read_param_size(&mut self) -> Result<u32> {
        self.reader.read_u32()
    }

    pub(crate) fn read_tpm2b(&mut self) -> Result<&'a [u8]> {
        self.reader.read_sized()
    }
}

/// Validates a response header and checks for TPM errors.
///
/// # Errors
///
/// Returns an error when the response is too short, has an unexpected tag, or carries a nonzero TPM return code.
pub(crate) fn validate(response: &[u8]) -> Result<()> {
    if response.len() < RESPONSE_HEADER_SIZE {
        return Err(Tpm2Error::ResponseTooShort {
            expected: RESPONSE_HEADER_SIZE,
            actual: response.len(),
        });
    }

    let mut reader = ResponseReader::new(response);
    let tag = reader.read_u16()?;

    if tag != TPM2_ST_NO_SESSIONS && tag != TPM2_ST_SESSIONS {
        return Err(Tpm2Error::BadResponseTag);
    }

    let _size = reader.read_u32()?;
    let rc = reader.read_u32()?;

    if rc != 0 {
        return Err(Tpm2Error::TpmError(rc));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::*;

    fn response(tag: u16, rc: u32, body: &[u8]) -> Vec<u8> {
        let size = 10_usize
            .checked_add(body.len())
            .expect("response size should fit usize");
        let mut out = Vec::with_capacity(size);
        out.extend_from_slice(&tag.to_be_bytes());
        let size = u32::try_from(size).ok().unwrap_or(0);
        out.extend_from_slice(&size.to_be_bytes());
        out.extend_from_slice(&rc.to_be_bytes());
        out.extend_from_slice(body);

        out
    }

    #[test]
    fn validate_accepts_success_tags() {
        // ARRANGE
        let no_sessions = response(TPM2_ST_NO_SESSIONS, 0, &[]);
        let sessions = response(TPM2_ST_SESSIONS, 0, &[]);

        // ACT
        let no_sessions_result = validate(&no_sessions);
        let sessions_result = validate(&sessions);

        // ASSERT
        assert!(
            no_sessions_result.is_ok(),
            "no-sessions response should validate"
        );
        assert!(sessions_result.is_ok(), "sessions response should validate");
    }

    #[test]
    fn validate_rejects_bad_headers() {
        // ARRANGE
        let short = [0_u8; 9];
        let bad_tag = response(0xFFFF, 0, &[]);
        let tpm_error = response(TPM2_ST_NO_SESSIONS, 0x101, &[]);

        // ACT
        let short_result = validate(&short);
        let bad_tag_result = validate(&bad_tag);
        let tpm_error_result = validate(&tpm_error);

        // ASSERT
        assert!(short_result.is_err(), "short response should fail");
        assert!(bad_tag_result.is_err(), "bad tag should fail");
        assert!(tpm_error_result.is_err(), "TPM error code should fail");
    }

    #[test]
    fn response_reader_reads_values_in_order() {
        // ARRANGE
        let data = [0x01, 0x02, 0x03, 0x04, 0x05, 0x00, 0x02, 0xAA, 0xBB];
        let mut reader = ResponseReader::new(&data);

        // ACT
        let byte = reader.read_u8();
        let word = reader.read_u16();
        let bytes = reader.read_bytes(2);
        let sized = reader.read_sized();

        // ASSERT
        assert!(byte.is_ok(), "u8 read should succeed");
        assert!(word.is_ok(), "u16 read should succeed");
        assert!(bytes.is_ok(), "bytes read should succeed");
        assert!(sized.is_ok(), "sized read should succeed");
        assert_eq!(byte.ok(), Some(0x01), "u8 should be read first");
        assert_eq!(word.ok(), Some(0x0203), "u16 should be big-endian");
        assert_eq!(
            bytes.ok(),
            Some(&[0x04, 0x05][..]),
            "raw bytes should match"
        );
        assert_eq!(
            sized.ok(),
            Some(&[0xAA, 0xBB][..]),
            "sized bytes should match"
        );
    }

    #[test]
    fn response_reader_reports_short_reads() {
        // ARRANGE
        let data = [0x01, 0x02, 0x03];
        let mut u16_reader = ResponseReader::new(&data[0..1]);
        let mut u32_reader = ResponseReader::new(&data);
        let mut bytes_reader = ResponseReader::new(&data);

        // ACT
        let u16_result = u16_reader.read_u16();
        let u32_result = u32_reader.read_u32();
        let bytes_result = bytes_reader.read_bytes(4);

        // ASSERT
        assert!(u16_result.is_err(), "short u16 should fail");
        assert!(u32_result.is_err(), "short u32 should fail");
        assert!(bytes_result.is_err(), "short byte slice should fail");
    }

    #[test]
    fn response_body_skips_response_header() {
        // ARRANGE
        let response = [0x80, 0x01, 0, 0, 0, 12, 0, 0, 0, 0, 0xAA, 0xBB];

        // ACT
        let body = ResponseBody::from_response(&response);

        // ASSERT
        assert!(body.is_ok(), "response body should skip response header");
    }

    #[test]
    fn response_body_rejects_short_response() {
        // ARRANGE
        let response = [0_u8; 9];

        // ACT
        let body = ResponseBody::from_response(&response);

        // ASSERT
        assert!(body.is_err(), "short response should fail");
    }
}
