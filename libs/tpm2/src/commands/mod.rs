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
