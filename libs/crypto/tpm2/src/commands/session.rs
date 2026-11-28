//! TPM session and transient-handle commands.

use super::TpmCommand;
use crate::buffer::CommandBuffer;
use crate::error::Result;
use crate::handles::{HierarchyHandle, SessionHandle};
use crate::pcr::pcr11_selection_bytes;
use crate::response::ResponseBody;
use crate::tags::TPM2_ST_NO_SESSIONS;

const TPM2_CC_FLUSH_CONTEXT: u32 = 0x0000_0165;
const TPM2_CC_START_AUTH_SESSION: u32 = 0x0000_0176;
const TPM2_CC_POLICY_PCR: u32 = 0x0000_017F;
const TPM2_ALG_SHA256: u16 = 0x000B;
const TPM2_ALG_NULL: u16 = 0x0010;
const TPM2_SE_POLICY: u8 = 0x01;

pub(crate) struct StartAuthSessionCommand<'a> {
    pub(crate) nonce: &'a [u8],
}

impl TpmCommand for StartAuthSessionCommand<'_> {
    type Output<'a> = SessionHandle;

    const TAG: u16 = TPM2_ST_NO_SESSIONS;
    const COMMAND_CODE: u32 = TPM2_CC_START_AUTH_SESSION;

    fn encode(&self, command: &mut CommandBuffer) -> Result<()> {
        command.write_handle(HierarchyHandle::NULL)?;
        command.write_handle(HierarchyHandle::NULL)?;
        command.write_sized(self.nonce)?;
        command.write_sized(&[])?;
        command.write_u8(TPM2_SE_POLICY)?;
        command.write_u16(TPM2_ALG_NULL)?;
        command.write_u16(TPM2_ALG_SHA256)?;
        Ok(())
    }

    fn decode<'a>(&self, body: &mut ResponseBody<'a>) -> Result<Self::Output<'a>> {
        body.read_handle::<SessionHandle>()
    }
}

pub(crate) struct PolicyPcrCommand<'a> {
    pub(crate) session_handle: SessionHandle,
    pub(crate) pcr_digest: &'a [u8],
}

impl TpmCommand for PolicyPcrCommand<'_> {
    type Output<'a> = ();

    const TAG: u16 = TPM2_ST_NO_SESSIONS;
    const COMMAND_CODE: u32 = TPM2_CC_POLICY_PCR;

    fn encode(&self, command: &mut CommandBuffer) -> Result<()> {
        command.write_handle(self.session_handle)?;
        command.write_sized(self.pcr_digest)?;
        command.write_bytes(pcr11_selection_bytes())?;
        Ok(())
    }

    fn decode<'a>(&self, _body: &mut ResponseBody<'a>) -> Result<Self::Output<'a>> {
        Ok(())
    }
}

pub(crate) struct FlushContextCommand<H> {
    pub(crate) handle: H,
}

impl<H> TpmCommand for FlushContextCommand<H>
where
    H: Copy + Into<u32>,
{
    type Output<'a> = ();

    const TAG: u16 = TPM2_ST_NO_SESSIONS;
    const COMMAND_CODE: u32 = TPM2_CC_FLUSH_CONTEXT;

    fn encode(&self, command: &mut CommandBuffer) -> Result<()> {
        command.write_handle(self.handle)?;
        Ok(())
    }

    fn decode<'a>(&self, _body: &mut ResponseBody<'a>) -> Result<Self::Output<'a>> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::*;
    use crate::handles::TransientHandle;

    fn response_bytes(body: &[u8]) -> Vec<u8> {
        let size = 10_usize
            .checked_add(body.len())
            .expect("response size should fit usize");
        let mut response = Vec::with_capacity(size);
        response.extend_from_slice(&TPM2_ST_NO_SESSIONS.to_be_bytes());
        response.extend_from_slice(&u32::try_from(size).unwrap_or(0).to_be_bytes());
        response.extend_from_slice(&0_u32.to_be_bytes());
        response.extend_from_slice(body);
        response
    }

    #[test]
    fn start_auth_session_encodes_and_decodes() {
        // ARRANGE
        let nonce = [0xAA_u8; 32];
        let command = StartAuthSessionCommand { nonce: &nonce };
        let mut buffer = CommandBuffer::new(
            StartAuthSessionCommand::TAG,
            StartAuthSessionCommand::COMMAND_CODE,
        );
        let response = response_bytes(&[0x03, 0x00, 0x00, 0x01]);
        let mut body = ResponseBody::from_response(&response).expect("response should parse");

        // ACT
        let encode_result = command.encode(&mut buffer);
        let finalized = buffer.finalize();
        let decode_result = command.decode(&mut body);

        // ASSERT
        assert!(encode_result.is_ok(), "start auth session should encode");
        assert!(finalized.is_ok(), "start auth session should finalize");
        let finalized = finalized.unwrap_or_default();
        assert_eq!(
            finalized.get(10..18),
            Some(&[0x40, 0x00, 0x00, 0x07, 0x40, 0x00, 0x00, 0x07][..]),
            "session should use null hierarchy handles"
        );
        assert_eq!(
            finalized.get(18..20),
            Some(32_u16.to_be_bytes().as_slice()),
            "nonce should be 32 bytes"
        );
        assert_eq!(
            finalized.get(52..57),
            Some(&[0x00, 0x00, TPM2_SE_POLICY, 0x00, 0x10][..]),
            "session parameters should match expected tail bytes"
        );
        assert_eq!(
            decode_result.ok(),
            Some(SessionHandle::from(0x0300_0001)),
            "session handle should decode"
        );
    }

    #[test]
    fn policy_pcr_and_flush_context_encode_and_decode() {
        // ARRANGE
        let policy = PolicyPcrCommand {
            session_handle: SessionHandle::from(0x0300_0000),
            pcr_digest: &[0xAA, 0xBB],
        };
        let flush = FlushContextCommand {
            handle: TransientHandle::from(0x8000_0003),
        };
        let mut policy_buffer =
            CommandBuffer::new(PolicyPcrCommand::TAG, PolicyPcrCommand::COMMAND_CODE);
        let mut flush_buffer = CommandBuffer::new(
            FlushContextCommand::<TransientHandle>::TAG,
            FlushContextCommand::<TransientHandle>::COMMAND_CODE,
        );
        let empty_response = response_bytes(&[]);
        let mut empty_body =
            ResponseBody::from_response(&empty_response).expect("response should parse");
        let another_empty_response = response_bytes(&[]);
        let mut another_empty_body =
            ResponseBody::from_response(&another_empty_response).expect("response should parse");

        // ACT
        let policy_encode = policy.encode(&mut policy_buffer);
        let flush_encode = flush.encode(&mut flush_buffer);
        let policy_finalized = policy_buffer.finalize();
        let flush_finalized = flush_buffer.finalize();
        let policy_decode = policy.decode(&mut empty_body);
        let flush_decode = flush.decode(&mut another_empty_body);

        // ASSERT
        assert!(policy_encode.is_ok(), "policy PCR should encode");
        assert!(flush_encode.is_ok(), "flush context should encode");
        assert!(policy_finalized.is_ok(), "policy PCR should finalize");
        assert!(flush_finalized.is_ok(), "flush context should finalize");
        let policy_finalized = policy_finalized.unwrap_or_default();
        let flush_finalized = flush_finalized.unwrap_or_default();
        assert_eq!(
            policy_finalized.get(10..14),
            Some(0x0300_0000_u32.to_be_bytes().as_slice()),
            "policy PCR should encode the session handle"
        );
        assert_eq!(
            policy_finalized.get(18..),
            Some(pcr11_selection_bytes().as_slice()),
            "policy PCR should encode PCR 11 selection"
        );
        assert_eq!(
            flush_finalized.get(10..14),
            Some(0x8000_0003_u32.to_be_bytes().as_slice()),
            "flush context should encode the target handle"
        );
        assert!(policy_decode.is_ok(), "policy PCR decode should succeed");
        assert!(flush_decode.is_ok(), "flush context decode should succeed");
    }
}
