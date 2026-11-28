//! TPM2 command buffer helpers.

use crate::error::{Result, Tpm2Error};

/// Maximum size in bytes of an encoded TPM2 command.
pub(crate) const MAX_COMMAND_SIZE: usize = 1024;
const HEADER_SIZE: usize = 10;
const SIZE_FIELD: core::ops::Range<usize> = 2..6;

/// Fixed-capacity marshalling buffer for building TPM2 commands.
pub(crate) struct CommandBuffer {
    buf: [u8; MAX_COMMAND_SIZE],
    len: usize,
}

impl CommandBuffer {
    #[must_use = "callers need the buffer to accumulate the command"]
    pub(crate) fn new(tag: u16, command_code: u32) -> Self {
        let mut buf = [0_u8; MAX_COMMAND_SIZE];
        let header = tag
            .to_be_bytes()
            .into_iter()
            .chain(0_u32.to_be_bytes())
            .chain(command_code.to_be_bytes());
        for (slot, byte) in buf.iter_mut().zip(header) {
            *slot = byte;
        }

        Self {
            buf,
            len: HEADER_SIZE,
        }
    }

    pub(crate) fn write_u8(&mut self, value: u8) -> Result<()> {
        self.push(&[value])
    }

    pub(crate) fn write_u16(&mut self, value: u16) -> Result<()> {
        self.push(&value.to_be_bytes())
    }

    pub(crate) fn write_u32(&mut self, value: u32) -> Result<()> {
        self.push(&value.to_be_bytes())
    }

    pub(crate) fn write_handle<H>(&mut self, value: H) -> Result<()>
    where
        H: Into<u32>,
    {
        self.write_u32(value.into())
    }

    pub(crate) fn write_bytes(&mut self, data: &[u8]) -> Result<()> {
        self.push(data)
    }

    pub(crate) fn write_sized(&mut self, data: &[u8]) -> Result<()> {
        self.write_u16(u16_len(data.len())?)?;
        self.push(data)
    }

    pub(crate) fn finalize(&mut self) -> Result<&[u8]> {
        let len = u32_len(self.len)?;
        let Some(size_bytes) = self.buf.get_mut(SIZE_FIELD) else {
            return Err(Tpm2Error::BufferTooLarge {
                actual: self.len,
                max: MAX_COMMAND_SIZE,
            });
        };
        size_bytes.copy_from_slice(&len.to_be_bytes());

        self.buf.get(..self.len).ok_or(Tpm2Error::BufferTooLarge {
            actual: self.len,
            max: MAX_COMMAND_SIZE,
        })
    }

    fn push(&mut self, bytes: &[u8]) -> Result<()> {
        let end = self
            .len
            .checked_add(bytes.len())
            .ok_or(Tpm2Error::BufferTooLarge {
                actual: bytes.len(),
                max: MAX_COMMAND_SIZE,
            })?;
        let Some(target) = self.buf.get_mut(self.len..end) else {
            return Err(Tpm2Error::BufferTooLarge {
                actual: end,
                max: MAX_COMMAND_SIZE,
            });
        };
        target.copy_from_slice(bytes);
        self.len = end;

        Ok(())
    }
}

pub(crate) fn u16_len(len: usize) -> Result<u16> {
    u16::try_from(len).map_err(|_err| Tpm2Error::BufferTooLarge {
        actual: len,
        max: usize::from(u16::MAX),
    })
}

pub(crate) fn u32_len(len: usize) -> Result<u32> {
    u32::try_from(len).map_err(|_err| Tpm2Error::BufferTooLarge {
        actual: len,
        max: usize::try_from(u32::MAX).unwrap_or(usize::MAX),
    })
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;
    use crate::tags::TPM2_ST_NO_SESSIONS;

    #[test]
    fn command_buffer_finalizes_header_size() {
        // ARRANGE
        let mut command = CommandBuffer::new(TPM2_ST_NO_SESSIONS, 0x0000_0165);
        command.write_u8(0xAA).expect("u8 write should succeed");
        command.write_u16(0xBBCC).expect("u16 write should succeed");
        command
            .write_u32(0xDDEE_FF00)
            .expect("u32 write should succeed");
        command
            .write_bytes(&[0x11, 0x22])
            .expect("byte write should succeed");
        let sized_write = command.write_sized(&[0x33, 0x44]);
        assert!(sized_write.is_ok(), "sized write should succeed");

        // ACT
        let finalized = command.finalize();

        // ASSERT
        assert!(finalized.is_ok(), "finalize should succeed");
        let finalized = finalized.ok().unwrap_or_default();
        assert_eq!(
            finalized.len(),
            23,
            "command should include header and payload"
        );
        assert_eq!(
            finalized.get(0..2),
            Some(TPM2_ST_NO_SESSIONS.to_be_bytes().as_slice()),
            "tag should match"
        );
        assert_eq!(
            finalized.get(2..6),
            Some(23_u32.to_be_bytes().as_slice()),
            "size should match"
        );
        assert_eq!(
            finalized.get(6..10),
            Some(0x0000_0165_u32.to_be_bytes().as_slice()),
            "command code should match",
        );
    }

    #[test]
    fn finalize_returns_full_command_slice_with_size_field() {
        // ARRANGE
        let payload = [0x5A_u8; MAX_COMMAND_SIZE - HEADER_SIZE];
        let mut command = CommandBuffer::new(TPM2_ST_NO_SESSIONS, 0x0000_0165);
        let written = command.write_bytes(&payload);
        let expected_size = u32::try_from(MAX_COMMAND_SIZE).ok().unwrap_or(0);

        // ACT
        let finalized = command.finalize();

        // ASSERT
        assert!(written.is_ok(), "payload should fill the command exactly");
        let finalized = finalized.expect("full command should finalize");
        assert_eq!(
            finalized.len(),
            MAX_COMMAND_SIZE,
            "command should use the whole buffer"
        );
        assert_eq!(
            finalized.get(2..6),
            Some(expected_size.to_be_bytes().as_slice()),
            "size field should match the written length"
        );
        assert_eq!(
            finalized.get(HEADER_SIZE..),
            Some(payload.as_slice()),
            "payload should round-trip unchanged"
        );
    }

    #[test]
    fn write_past_max_command_size_is_rejected() {
        // ARRANGE
        let payload = [0x5A_u8; MAX_COMMAND_SIZE - HEADER_SIZE];
        let mut command = CommandBuffer::new(TPM2_ST_NO_SESSIONS, 0x0000_0165);
        command
            .write_bytes(&payload)
            .expect("payload should fill the command exactly");
        let overflow = [0xFF_u8; 4];
        let rejected_len = MAX_COMMAND_SIZE + overflow.len();

        // ACT
        let result = command.write_bytes(&overflow);
        let empty_write = command.write_bytes(&[]);
        let finalized = command.finalize();

        // ASSERT
        assert!(
            matches!(
                result,
                Err(Tpm2Error::BufferTooLarge { actual, max })
                    if max == MAX_COMMAND_SIZE && actual == rejected_len
            ),
            "overflow should report the buffer limit and the length the write needed"
        );
        assert!(
            empty_write.is_ok(),
            "zero-length write should succeed on a full buffer"
        );
        let finalized = finalized.expect("rejected write should leave the command valid");
        assert_eq!(
            finalized.len(),
            MAX_COMMAND_SIZE,
            "rejected write should not change the command length"
        );
        assert_eq!(
            finalized.get(HEADER_SIZE..),
            Some(payload.as_slice()),
            "rejected write should leave the finalized slice untouched"
        );
    }

    #[test]
    fn write_sized_rejects_oversized_payload() {
        // ARRANGE
        let oversized = vec![0_u8; usize::from(u16::MAX) + 1];
        let mut command = CommandBuffer::new(TPM2_ST_NO_SESSIONS, 0x0000_0165);

        // ACT
        let result = command.write_sized(&oversized);

        // ASSERT
        assert!(result.is_err(), "oversized sized buffer should fail");
    }

    #[test]
    fn conversion_helpers_reject_large_values() {
        // ACT
        let u16_result = u16_len(usize::from(u16::MAX) + 1);
        let u32_result = u32_len(usize::MAX);

        // ASSERT
        assert!(u16_result.is_err(), "large u16 conversion should fail");
        assert_eq!(
            u32_result.is_err(),
            usize::BITS > u32::BITS,
            "u32 conversion should match target width"
        );
    }
}
