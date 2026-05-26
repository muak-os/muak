//! TPM2 command buffer helpers.

use crate::error::{Result, Tpm2Error};

/// Marshalling buffer for building TPM2 commands.
pub struct CommandBuffer {
    buf: Vec<u8>,
}

impl CommandBuffer {
    #[must_use]
    pub fn new(tag: u16, command_code: u32) -> Self {
        let mut buf = Vec::with_capacity(256);
        buf.extend_from_slice(&tag.to_be_bytes());
        buf.extend_from_slice(&0_u32.to_be_bytes());
        buf.extend_from_slice(&command_code.to_be_bytes());
        Self { buf }
    }

    pub fn write_u8(&mut self, value: u8) {
        self.buf.push(value);
    }

    pub fn write_u16(&mut self, value: u16) {
        self.buf.extend_from_slice(&value.to_be_bytes());
    }

    pub fn write_u32(&mut self, value: u32) {
        self.buf.extend_from_slice(&value.to_be_bytes());
    }

    pub fn write_handle<H>(&mut self, value: H)
    where
        H: Into<u32>,
    {
        self.write_u32(value.into());
    }

    pub fn write_bytes(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    pub fn write_sized(&mut self, data: &[u8]) -> Result<()> {
        self.write_u16(u16_len(data.len())?);
        self.buf.extend_from_slice(data);
        Ok(())
    }

    pub fn finalize(mut self) -> Result<Vec<u8>> {
        let len = u32_len(self.buf.len())?;
        if let Some(size_bytes) = self.buf.get_mut(2..6) {
            size_bytes.copy_from_slice(&len.to_be_bytes());
        }
        Ok(self.buf)
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
    use super::*;

    #[test]
    fn command_buffer_finalizes_header_size() {
        // ARRANGE
        let mut command = CommandBuffer::new(0x8001, 0x0000_0165);
        command.write_u8(0xAA);
        command.write_u16(0xBBCC);
        command.write_u32(0xDDEE_FF00);
        command.write_bytes(&[0x11, 0x22]);
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
            Some(0x8001_u16.to_be_bytes().as_slice()),
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
    fn write_sized_rejects_oversized_payload() {
        // ARRANGE
        let oversized = vec![0_u8; usize::from(u16::MAX) + 1];
        let mut command = CommandBuffer::new(0x8001, 0x0000_0165);

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
