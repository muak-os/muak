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
