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
