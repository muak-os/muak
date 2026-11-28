//! TPM2 command marshalling and execution.

mod object;
mod persistence;
mod session;
mod template;

pub(crate) use object::{CreateCommand, CreatePrimaryCommand, LoadCommand, UnsealCommand};
pub(crate) use persistence::{EvictControlCommand, HandleExistsCommand};
pub(crate) use session::{FlushContextCommand, PolicyPcrCommand, StartAuthSessionCommand};

use crate::buffer::CommandBuffer;
use crate::error::Result;
use crate::response::{self, ResponseBody};
use crate::transport::Transport;

/// A TPM command that can be encoded, sent over a `Transport`, and decoded.
pub(crate) trait TpmCommand {
    const COMMAND_CODE: u32;
    const TAG: u16;

    type Output;

    fn decode(&self, body: &mut ResponseBody<'_>) -> Result<Self::Output>;

    fn encode(&self, command: &mut CommandBuffer) -> Result<()>;
}

/// Execute a TPM command by encoding it, sending it to the transport, validating, and decoding the response.
pub(crate) fn execute<T, C>(transport: &mut T, command: &C) -> Result<C::Output>
where
    T: Transport,
    C: TpmCommand,
{
    let mut buffer = CommandBuffer::new(C::TAG, C::COMMAND_CODE);
    command.encode(&mut buffer)?;
    let response = transport.transact(&buffer.finalize()?)?;
    response::validate(response)?;
    let mut body = ResponseBody::from_response(response)?;

    command.decode(&mut body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Tpm2Error;
    use crate::tags::TPM2_ST_NO_SESSIONS;
    use crate::transport::Transport;

    #[derive(Default)]
    struct MockTransport {
        response: Vec<u8>,
        written: Vec<u8>,
    }

    impl Transport for MockTransport {
        fn transact(&mut self, request: &[u8]) -> Result<&[u8]> {
            self.written.extend_from_slice(request);
            Ok(&self.response)
        }
    }

    struct MockCommand;

    impl TpmCommand for MockCommand {
        type Output = u32;

        const TAG: u16 = TPM2_ST_NO_SESSIONS;
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

        const TAG: u16 = TPM2_ST_NO_SESSIONS;
        const COMMAND_CODE: u32 = 0x0000_00EE;

        fn encode(&self, _command: &mut CommandBuffer) -> Result<()> {
            Err(Tpm2Error::InvalidBlob)
        }

        fn decode(&self, _body: &mut ResponseBody<'_>) -> Result<Self::Output> {
            Ok(())
        }
    }

    fn response_with(tag: u16, rc: u32, body: &[u8]) -> Vec<u8> {
        let size = 10_usize
            .checked_add(body.len())
            .expect("response size should fit usize");
        let mut out = Vec::with_capacity(size);
        out.extend_from_slice(&tag.to_be_bytes());
        out.extend_from_slice(&u32::try_from(size).unwrap_or(0).to_be_bytes());
        out.extend_from_slice(&rc.to_be_bytes());
        out.extend_from_slice(body);
        out
    }

    fn response(body: &[u8]) -> Vec<u8> {
        response_with(TPM2_ST_NO_SESSIONS, 0, body)
    }

    #[test]
    fn execute_encodes_transacts_and_decodes() {
        // ARRANGE
        let mut transport = MockTransport {
            response: response(&0xDEAD_BEEF_u32.to_be_bytes()),
            written: Vec::new(),
        };

        // ACT
        let result = execute(&mut transport, &MockCommand);

        // ASSERT
        assert_eq!(result.ok(), Some(0xDEAD_BEEF), "decoded value should match");
        assert_eq!(
            transport.written.get(0..2),
            Some(TPM2_ST_NO_SESSIONS.to_be_bytes().as_slice()),
            "command tag should be written"
        );
        assert_eq!(
            transport.written.get(6..10),
            Some(0x0000_00FF_u32.to_be_bytes().as_slice()),
            "command code should be written"
        );
    }

    #[test]
    fn execute_propagates_encode_errors() {
        // ARRANGE
        let mut transport = MockTransport::default();

        // ACT
        let result = execute(&mut transport, &FailingCommand);

        // ASSERT
        assert!(result.is_err(), "encode failure should be propagated");
        assert!(
            transport.written.is_empty(),
            "transport should not be used on encode failure"
        );
    }

    #[test]
    fn execute_rejects_invalid_responses() {
        // ARRANGE
        let mut error_transport = MockTransport {
            response: response_with(TPM2_ST_NO_SESSIONS, 0x101, &[]),
            written: Vec::new(),
        };
        let mut bad_tag_transport = MockTransport {
            response: response_with(0xFFFF, 0, &[]),
            written: Vec::new(),
        };

        // ACT
        let error_result = execute(&mut error_transport, &MockCommand);
        let bad_tag_result = execute(&mut bad_tag_transport, &MockCommand);

        // ASSERT
        assert!(
            error_result.is_err(),
            "non-zero return code should be rejected"
        );
        assert!(bad_tag_result.is_err(), "bad tag should be rejected");
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
