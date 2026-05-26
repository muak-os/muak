//! TPM persistence and capability commands.

use super::TpmCommand;
use crate::auth::AuthArea;
use crate::buffer::CommandBuffer;
use crate::error::Result;
use crate::handles::{HierarchyHandle, PersistentHandle, TransientHandle};
use crate::response::ResponseBody;

const TPM2_ST_NO_SESSIONS: u16 = 0x8001;
const TPM2_ST_SESSIONS: u16 = 0x8002;
const TPM2_CC_EVICT_CONTROL: u32 = 0x0000_0120;
const TPM2_CC_GET_CAPABILITY: u32 = 0x0000_017A;
const TPM2_CAP_HANDLES: u32 = 0x0000_0001;

pub(crate) struct HandleExistsCommand {
    pub(crate) handle: PersistentHandle,
}

impl TpmCommand for HandleExistsCommand {
    type Output = bool;

    const TAG: u16 = TPM2_ST_NO_SESSIONS;
    const COMMAND_CODE: u32 = TPM2_CC_GET_CAPABILITY;

    fn encode(&self, command: &mut CommandBuffer) -> Result<()> {
        command.write_u32(TPM2_CAP_HANDLES);
        command.write_handle(self.handle);
        command.write_u32(1);
        Ok(())
    }

    fn decode(&self, body: &mut ResponseBody<'_>) -> Result<Self::Output> {
        let _more = body.read_u8()?;
        let _cap = body.read_u32()?;
        let count = body.read_u32()?;

        if count == 0 {
            return Ok(false);
        }

        let found_handle = body.read_handle::<PersistentHandle>()?;
        Ok(found_handle == self.handle)
    }
}

pub(crate) struct EvictControlCommand {
    pub(crate) auth: HierarchyHandle,
    pub(crate) object: TransientHandle,
    pub(crate) persistent: PersistentHandle,
}

impl TpmCommand for EvictControlCommand {
    type Output = ();

    const TAG: u16 = TPM2_ST_SESSIONS;
    const COMMAND_CODE: u32 = TPM2_CC_EVICT_CONTROL;

    fn encode(&self, command: &mut CommandBuffer) -> Result<()> {
        command.write_handle(self.auth);
        command.write_handle(self.object);
        AuthArea::Password.encode_sized(command);
        command.write_handle(self.persistent);
        Ok(())
    }

    fn decode(&self, _body: &mut ResponseBody<'_>) -> Result<Self::Output> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response_body(tag: u16, body: &[u8]) -> ResponseBody<'_> {
        let size = 10 + body.len();
        let mut response = Vec::with_capacity(size);
        response.extend_from_slice(&tag.to_be_bytes());
        response.extend_from_slice(&u32::try_from(size).unwrap_or(0).to_be_bytes());
        response.extend_from_slice(&0_u32.to_be_bytes());
        response.extend_from_slice(body);
        let leaked = Box::leak(response.into_boxed_slice());
        match ResponseBody::from_response(leaked) {
            Ok(body) => body,
            Err(_) => panic!("response should parse"),
        }
    }

    #[test]
    fn handle_exists_command_encodes_and_decodes() {
        // ARRANGE
        let command = HandleExistsCommand {
            handle: PersistentHandle::new(0x8100_0001),
        };
        let mut buffer =
            CommandBuffer::new(HandleExistsCommand::TAG, HandleExistsCommand::COMMAND_CODE);
        let mut found_body = response_body(
            TPM2_ST_NO_SESSIONS,
            &[
                0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x81, 0x00, 0x00, 0x01,
            ],
        );
        let mut missing_body = response_body(
            TPM2_ST_NO_SESSIONS,
            &[0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00],
        );

        // ACT
        let encode_result = command.encode(&mut buffer);
        let finalized = buffer.finalize();
        let found = command.decode(&mut found_body);
        let missing = command.decode(&mut missing_body);

        // ASSERT
        assert!(encode_result.is_ok(), "handle exists should encode");
        assert!(finalized.is_ok(), "handle exists should finalize");
        assert_eq!(found.ok(), Some(true), "matching handle should be detected");
        assert_eq!(
            missing.ok(),
            Some(false),
            "empty handle list should be false"
        );
    }

    #[test]
    fn evict_control_command_encodes_and_decodes() {
        // ARRANGE
        let command = EvictControlCommand {
            auth: HierarchyHandle::OWNER,
            object: TransientHandle::from(0x8000_0001),
            persistent: PersistentHandle::new(0x8100_0001),
        };
        let mut buffer =
            CommandBuffer::new(EvictControlCommand::TAG, EvictControlCommand::COMMAND_CODE);
        let mut body = response_body(TPM2_ST_SESSIONS, &[]);

        // ACT
        let encode_result = command.encode(&mut buffer);
        let finalized = buffer.finalize();
        let decode_result = command.decode(&mut body);

        // ASSERT
        assert!(encode_result.is_ok(), "evict control should encode");
        assert!(finalized.is_ok(), "evict control should finalize");
        let finalized = finalized.unwrap_or_default();
        assert_eq!(
            finalized.get(10..18),
            Some(&[0x40, 0x00, 0x00, 0x01, 0x80, 0x00, 0x00, 0x01][..]),
            "evict control should encode auth and transient handles"
        );
        assert_eq!(
            finalized.get(finalized.len().saturating_sub(4)..),
            Some(0x8100_0001_u32.to_be_bytes().as_slice()),
            "evict control should encode the persistent handle"
        );
        assert!(decode_result.is_ok(), "evict control decode should succeed");
    }
}
