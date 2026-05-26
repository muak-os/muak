//! TPM session and transient-handle commands.

use ring::rand::{SecureRandom as _, SystemRandom};

use super::TpmCommand;
use crate::buffer::CommandBuffer;
use crate::error::{Result, Tpm2Error};
use crate::handles::{HierarchyHandle, SessionHandle};
use crate::pcr::pcr11_selection_bytes;
use crate::response::ResponseBody;

const TPM2_ST_NO_SESSIONS: u16 = 0x8001;
const TPM2_CC_FLUSH_CONTEXT: u32 = 0x0000_0165;
const TPM2_CC_START_AUTH_SESSION: u32 = 0x0000_0176;
const TPM2_CC_POLICY_PCR: u32 = 0x0000_017F;
const TPM2_ALG_SHA256: u16 = 0x000B;
const TPM2_ALG_NULL: u16 = 0x0010;
const TPM2_SE_POLICY: u8 = 0x01;

pub(crate) struct StartAuthSessionCommand;

impl TpmCommand for StartAuthSessionCommand {
    type Output = SessionHandle;

    const TAG: u16 = TPM2_ST_NO_SESSIONS;
    const COMMAND_CODE: u32 = TPM2_CC_START_AUTH_SESSION;

    fn encode(&self, command: &mut CommandBuffer) -> Result<()> {
        let rng = SystemRandom::new();
        let mut nonce = [0_u8; 16];
        rng.fill(&mut nonce)
            .map_err(|_rng_error| Tpm2Error::RngFailed)?;

        command.write_handle(HierarchyHandle::NULL);
        command.write_handle(HierarchyHandle::NULL);
        command.write_sized(&nonce)?;
        command.write_sized(&[])?;
        command.write_u8(TPM2_SE_POLICY);
        command.write_u16(TPM2_ALG_NULL);
        command.write_u16(TPM2_ALG_SHA256);
        Ok(())
    }

    fn decode(&self, body: &mut ResponseBody<'_>) -> Result<Self::Output> {
        body.read_handle::<SessionHandle>()
    }
}

pub(crate) struct PolicyPcrCommand<'a> {
    pub(crate) session_handle: SessionHandle,
    pub(crate) pcr_digest: &'a [u8],
}

impl TpmCommand for PolicyPcrCommand<'_> {
    type Output = ();

    const TAG: u16 = TPM2_ST_NO_SESSIONS;
    const COMMAND_CODE: u32 = TPM2_CC_POLICY_PCR;

    fn encode(&self, command: &mut CommandBuffer) -> Result<()> {
        command.write_handle(self.session_handle);
        command.write_sized(self.pcr_digest)?;
        command.write_bytes(pcr11_selection_bytes());
        Ok(())
    }

    fn decode(&self, _body: &mut ResponseBody<'_>) -> Result<Self::Output> {
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
    type Output = ();

    const TAG: u16 = TPM2_ST_NO_SESSIONS;
    const COMMAND_CODE: u32 = TPM2_CC_FLUSH_CONTEXT;

    fn encode(&self, command: &mut CommandBuffer) -> Result<()> {
        command.write_handle(self.handle);
        Ok(())
    }

    fn decode(&self, _body: &mut ResponseBody<'_>) -> Result<Self::Output> {
        Ok(())
    }
}
