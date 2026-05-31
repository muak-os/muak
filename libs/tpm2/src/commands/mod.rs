//! TPM2 command marshalling and execution.

mod object;
mod persistence;
mod session;
mod template;

pub(crate) use object::{CreateCommand, CreatePrimaryCommand, LoadCommand, UnsealCommand};
pub(crate) use persistence::{EvictControlCommand, HandleExistsCommand};
pub(crate) use session::{FlushContextCommand, PolicyPcrCommand, StartAuthSessionCommand};

use crate::buffer::CommandBuffer;
use crate::device::TpmDevice;
use crate::error::Result;
use crate::response::ResponseBody;

/// A TPM command that can be encoded, sent to the device, and decoded.
pub(crate) trait TpmCommand {
    type Output;

    const TAG: u16;
    const COMMAND_CODE: u32;

    fn encode(&self, command: &mut CommandBuffer) -> Result<()>;

    fn decode(&self, body: &mut ResponseBody<'_>) -> Result<Self::Output>;
}

/// Execute a TPM command by encoding it, sending it to the device, and decoding the response.
pub(crate) fn execute<T, C>(dev: &mut T, command: &C) -> Result<C::Output>
where
    T: TpmDevice,
    C: TpmCommand,
{
    let mut buffer = CommandBuffer::new(C::TAG, C::COMMAND_CODE);
    command.encode(&mut buffer)?;
    let response = dev.transact(&buffer.finalize()?)?;
    let mut body = ResponseBody::from_response(&response)?;
    command.decode(&mut body)
}

#[cfg(test)]
mod tests {
    use std::io::{Error as IoError, ErrorKind, Read, Result as IoResult, Write};

    use super::*;
    use crate::error::Tpm2Error;

    #[derive(Default)]
    struct MockDevice {
        response: Vec<u8>,
        written: Vec<u8>,
    }

    impl Read for MockDevice {
        fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
            let read_len = buf.len().min(self.response.len());
            let target = buf
                .get_mut(..read_len)
                .ok_or_else(|| IoError::new(ErrorKind::UnexpectedEof, "short read buffer"))?;
            let source = self
                .response
                .get(..read_len)
                .ok_or_else(|| IoError::new(ErrorKind::UnexpectedEof, "short mock response"))?;
            target.copy_from_slice(source);
            self.response.drain(..read_len);
            Ok(read_len)
        }
    }

    impl Write for MockDevice {
        fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
            self.written.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> IoResult<()> {
            Ok(())
        }
    }

    impl TpmDevice for MockDevice {}

    struct MockCommand;

    impl TpmCommand for MockCommand {
        type Output = u32;

        const TAG: u16 = 0x8001;
        const COMMAND_CODE: u32 = 0x0000_00FF;

        fn encode(&self, command: &mut CommandBuffer) -> Result<()> {
            command.write_u16(0xABCD);
            Ok(())
        }

        fn decode(&self, body: &mut ResponseBody<'_>) -> Result<Self::Output> {
            body.read_u32()
        }
    }

    struct FailingCommand;

    impl TpmCommand for FailingCommand {
        type Output = ();

        const TAG: u16 = 0x8001;
        const COMMAND_CODE: u32 = 0x0000_00EE;

        fn encode(&self, _command: &mut CommandBuffer) -> Result<()> {
            Err(Tpm2Error::InvalidBlob)
        }

        fn decode(&self, _body: &mut ResponseBody<'_>) -> Result<Self::Output> {
            Ok(())
        }
    }

    fn response(body: &[u8]) -> Vec<u8> {
        let size = 10_usize
            .checked_add(body.len())
            .expect("response size should fit usize");
        let mut out = Vec::with_capacity(size);
        out.extend_from_slice(&0x8001_u16.to_be_bytes());
        out.extend_from_slice(&u32::try_from(size).unwrap_or(0).to_be_bytes());
        out.extend_from_slice(&0_u32.to_be_bytes());
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn execute_encodes_transacts_and_decodes() {
        // ARRANGE
        let mut dev = MockDevice {
            response: response(&0xDEAD_BEEF_u32.to_be_bytes()),
            written: Vec::new(),
        };

        // ACT
        let result = execute(&mut dev, &MockCommand);

        // ASSERT
        assert_eq!(result.ok(), Some(0xDEAD_BEEF), "decoded value should match");
        assert_eq!(
            dev.written.get(0..2),
            Some(0x8001_u16.to_be_bytes().as_slice()),
            "command tag should be written"
        );
        assert_eq!(
            dev.written.get(6..10),
            Some(0x0000_00FF_u32.to_be_bytes().as_slice()),
            "command code should be written"
        );
    }

    #[test]
    fn execute_propagates_encode_errors() {
        // ARRANGE
        let mut dev = MockDevice::default();

        // ACT
        let result = execute(&mut dev, &FailingCommand);

        // ASSERT
        assert!(result.is_err(), "encode failure should be propagated");
        assert!(
            dev.written.is_empty(),
            "device should not be used on encode failure"
        );
    }

    #[test]
    fn failing_command_decode_is_reachable() {
        // ARRANGE
        let mut body = ResponseBody::from_response(&[
            0x80, 0x01, 0x00, 0x00, 0x00, 0x0A, 0x00, 0x00, 0x00, 0x00,
        ])
        .expect("response should parse");

        // ACT
        let result = FailingCommand.decode(&mut body);

        // ASSERT
        assert!(result.is_ok(), "decode stub should be callable");
    }
}
