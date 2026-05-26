//! TPM object commands.

use zeroize::Zeroizing;

use super::TpmCommand;
use super::template::{write_seal_template, write_sensitive_create, write_srk_template};
use crate::auth::AuthArea;
use crate::buffer::CommandBuffer;
use crate::error::Result;
use crate::handles::{HierarchyHandle, PersistentHandle, SessionHandle, TransientHandle};
use crate::response::ResponseBody;

const TPM2_ST_SESSIONS: u16 = 0x8002;
const TPM2_CC_CREATE_PRIMARY: u32 = 0x0000_0131;
const TPM2_CC_CREATE: u32 = 0x0000_0153;
const TPM2_CC_LOAD: u32 = 0x0000_0157;
const TPM2_CC_UNSEAL: u32 = 0x0000_015E;

pub(crate) struct CreatePrimaryCommand;

impl TpmCommand for CreatePrimaryCommand {
    type Output = (TransientHandle, Vec<u8>);

    const TAG: u16 = TPM2_ST_SESSIONS;
    const COMMAND_CODE: u32 = TPM2_CC_CREATE_PRIMARY;

    fn encode(&self, command: &mut CommandBuffer) -> Result<()> {
        command.write_handle(HierarchyHandle::OWNER);
        AuthArea::Password.encode_sized(command);
        write_sensitive_create(command, &[], &[])?;
        write_srk_template(command);
        command.write_sized(&[])?;
        command.write_u32(0);
        Ok(())
    }

    fn decode(&self, body: &mut ResponseBody<'_>) -> Result<Self::Output> {
        let handle = body.read_handle()?;
        let _param_size = body.read_param_size()?;
        let pub_data = body.read_tpm2b()?;
        Ok((handle, pub_data.to_vec()))
    }
}

pub(crate) struct CreateCommand<'a> {
    pub(crate) parent_handle: PersistentHandle,
    pub(crate) policy_digest: &'a [u8],
    pub(crate) data: &'a [u8],
}

impl TpmCommand for CreateCommand<'_> {
    type Output = (Vec<u8>, Vec<u8>);

    const TAG: u16 = TPM2_ST_SESSIONS;
    const COMMAND_CODE: u32 = TPM2_CC_CREATE;

    fn encode(&self, command: &mut CommandBuffer) -> Result<()> {
        command.write_handle(self.parent_handle);
        AuthArea::Password.encode_sized(command);
        write_sensitive_create(command, &[], self.data)?;
        write_seal_template(command, self.policy_digest)?;
        command.write_sized(&[])?;
        command.write_u32(0);
        Ok(())
    }

    fn decode(&self, body: &mut ResponseBody<'_>) -> Result<Self::Output> {
        let _param_size = body.read_param_size()?;
        let priv_data = body.read_tpm2b()?.to_vec();
        let pub_data = body.read_tpm2b()?.to_vec();
        Ok((pub_data, priv_data))
    }
}

pub(crate) struct LoadCommand<'a> {
    pub(crate) parent_handle: PersistentHandle,
    pub(crate) pub_data: &'a [u8],
    pub(crate) priv_data: &'a [u8],
}

impl TpmCommand for LoadCommand<'_> {
    type Output = TransientHandle;

    const TAG: u16 = TPM2_ST_SESSIONS;
    const COMMAND_CODE: u32 = TPM2_CC_LOAD;

    fn encode(&self, command: &mut CommandBuffer) -> Result<()> {
        command.write_handle(self.parent_handle);
        AuthArea::Password.encode_sized(command);
        command.write_sized(self.priv_data)?;
        command.write_sized(self.pub_data)?;
        Ok(())
    }

    fn decode(&self, body: &mut ResponseBody<'_>) -> Result<Self::Output> {
        body.read_handle::<TransientHandle>()
    }
}

pub(crate) struct UnsealCommand {
    pub(crate) object_handle: TransientHandle,
    pub(crate) session_handle: SessionHandle,
}

impl TpmCommand for UnsealCommand {
    type Output = Zeroizing<Vec<u8>>;

    const TAG: u16 = TPM2_ST_SESSIONS;
    const COMMAND_CODE: u32 = TPM2_CC_UNSEAL;

    fn encode(&self, command: &mut CommandBuffer) -> Result<()> {
        command.write_handle(self.object_handle);
        AuthArea::Policy(self.session_handle).encode_sized(command);
        Ok(())
    }

    fn decode(&self, body: &mut ResponseBody<'_>) -> Result<Self::Output> {
        let _param_size = body.read_param_size()?;
        let data = body.read_tpm2b()?;
        Ok(Zeroizing::new(data.to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response_body(body: &[u8]) -> ResponseBody<'_> {
        let size = 10 + body.len();
        let mut response = Vec::with_capacity(size);
        response.extend_from_slice(&TPM2_ST_SESSIONS.to_be_bytes());
        response.extend_from_slice(&u32::try_from(size).unwrap_or(0).to_be_bytes());
        response.extend_from_slice(&0_u32.to_be_bytes());
        response.extend_from_slice(body);
        let leaked = Box::leak(response.into_boxed_slice());
        ResponseBody::from_response(leaked).expect("response should parse")
    }

    #[test]
    fn create_primary_command_encodes_and_decodes() {
        // ARRANGE
        let command = CreatePrimaryCommand;
        let mut buffer = CommandBuffer::new(
            CreatePrimaryCommand::TAG,
            CreatePrimaryCommand::COMMAND_CODE,
        );
        let mut body = response_body(&[
            0x80, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x05, 0x00, 0x02, 0xAA, 0xBB,
        ]);

        // ACT
        let encode_result = command.encode(&mut buffer);
        let finalized = buffer.finalize();
        let decode_result = command.decode(&mut body);

        // ASSERT
        assert!(encode_result.is_ok(), "create primary should encode");
        assert!(finalized.is_ok(), "encoded create primary should finalize");
        assert_eq!(
            decode_result.ok(),
            Some((TransientHandle::from(0x8000_0001), vec![0xAA, 0xBB])),
            "create primary should decode handle and public area"
        );
    }

    #[test]
    fn create_command_encodes_and_decodes() {
        // ARRANGE
        let command = CreateCommand {
            parent_handle: PersistentHandle::new(0x8100_0001),
            policy_digest: &[0x11, 0x22],
            data: &[0x33, 0x44],
        };
        let mut buffer = CommandBuffer::new(CreateCommand::TAG, CreateCommand::COMMAND_CODE);
        let mut body = response_body(&[
            0x00, 0x00, 0x00, 0x07, 0x00, 0x02, 0x10, 0x20, 0x00, 0x03, 0x30, 0x40, 0x50,
        ]);

        // ACT
        let encode_result = command.encode(&mut buffer);
        let finalized = buffer.finalize();
        let decode_result = command.decode(&mut body);

        // ASSERT
        assert!(encode_result.is_ok(), "create should encode");
        assert!(finalized.is_ok(), "encoded create should finalize");
        assert_eq!(
            decode_result.ok(),
            Some((vec![0x30, 0x40, 0x50], vec![0x10, 0x20])),
            "create should decode public and private blobs"
        );
    }

    #[test]
    fn load_and_unseal_commands_encode_and_decode() {
        // ARRANGE
        let load = LoadCommand {
            parent_handle: PersistentHandle::new(0x8100_0001),
            pub_data: &[0xAA],
            priv_data: &[0xBB, 0xCC],
        };
        let unseal = UnsealCommand {
            object_handle: TransientHandle::from(0x8000_0002),
            session_handle: SessionHandle::from(0x0300_0000),
        };
        let mut load_buffer = CommandBuffer::new(LoadCommand::TAG, LoadCommand::COMMAND_CODE);
        let mut unseal_buffer = CommandBuffer::new(UnsealCommand::TAG, UnsealCommand::COMMAND_CODE);
        let mut load_body = response_body(&[0x80, 0x00, 0x00, 0x02]);
        let mut unseal_body =
            response_body(&[0x00, 0x00, 0x00, 0x04, 0x00, 0x03, 0x01, 0x02, 0x03]);

        // ACT
        let load_encode = load.encode(&mut load_buffer);
        let unseal_encode = unseal.encode(&mut unseal_buffer);
        let load_finalized = load_buffer.finalize();
        let unseal_finalized = unseal_buffer.finalize();
        let load_decode = load.decode(&mut load_body);
        let unseal_decode = unseal.decode(&mut unseal_body);

        // ASSERT
        assert!(load_encode.is_ok(), "load should encode");
        assert!(unseal_encode.is_ok(), "unseal should encode");
        assert!(load_finalized.is_ok(), "load command should finalize");
        assert!(unseal_finalized.is_ok(), "unseal command should finalize");
        assert_eq!(
            load_decode.ok(),
            Some(TransientHandle::from(0x8000_0002)),
            "load should decode object handle"
        );
        let unseal_decode = unseal_decode.expect("unseal should decode returned data");
        assert_eq!(
            &unseal_decode[..],
            [1, 2, 3].as_slice(),
            "unseal should decode returned data"
        );
    }

    #[test]
    fn object_commands_propagate_encode_and_decode_errors() {
        // ARRANGE
        let oversized = vec![0_u8; usize::from(u16::MAX) + 1];
        let create = CreateCommand {
            parent_handle: PersistentHandle::new(0x8100_0001),
            policy_digest: &oversized,
            data: &[],
        };
        let load = LoadCommand {
            parent_handle: PersistentHandle::new(0x8100_0001),
            pub_data: &[],
            priv_data: &oversized,
        };
        let mut create_buffer = CommandBuffer::new(CreateCommand::TAG, CreateCommand::COMMAND_CODE);
        let mut load_buffer = CommandBuffer::new(LoadCommand::TAG, LoadCommand::COMMAND_CODE);
        let mut short_body = response_body(&[]);
        let mut short_unseal_body = response_body(&[0x00, 0x00, 0x00, 0x04]);

        // ACT
        let create_encode = create.encode(&mut create_buffer);
        let load_encode = load.encode(&mut load_buffer);
        let create_primary_decode = CreatePrimaryCommand.decode(&mut short_body);
        let create_decode = create.decode(&mut short_body);
        let load_decode = load.decode(&mut short_body);
        let unseal_decode = UnsealCommand {
            object_handle: TransientHandle::from(0x8000_0002),
            session_handle: SessionHandle::from(0x0300_0000),
        }
        .decode(&mut short_unseal_body);

        // ASSERT
        assert!(
            create_encode.is_err(),
            "oversized policy digest should fail create encode"
        );
        assert!(
            load_encode.is_err(),
            "oversized private data should fail load encode"
        );
        assert!(
            create_primary_decode.is_err(),
            "short create-primary response should fail"
        );
        assert!(create_decode.is_err(), "short create response should fail");
        assert!(load_decode.is_err(), "short load response should fail");
        assert!(unseal_decode.is_err(), "short unseal response should fail");
    }
}
