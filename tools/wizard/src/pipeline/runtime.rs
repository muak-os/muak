//! Runtime stream handles.

use std::io::{self, Write};
use std::os::unix::net::UnixStream;

use crate::error::{Result, WizardError};
use crate::pipeline::node::PortId;

/// A live input pipe end with the stream's fixed size.
pub(crate) struct InputStream<'a> {
    pub(crate) name: &'a str,
    pub(crate) size: u64,
    pub(crate) reader: UnixStream,
}

/// A live output end with the stream's fixed size.
pub(crate) struct OutputStream<'name, 'writer> {
    pub(crate) name: &'name str,
    pub(crate) size: u64,
    pub(crate) writer: OutputWriter<'writer>,
}

/// The final destination of an output stream:.
pub(crate) enum OutputWriter<'writer> {
    /// Pipe end read by the next node.
    Pipe(UnixStream),
    /// User writer consuming the stream's bytes.
    Target(&'writer mut (dyn Write + Send)),
}

impl Write for OutputWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match *self {
            Self::Pipe(ref mut writer) => writer.write(buf),
            Self::Target(ref mut writer) => writer.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match *self {
            Self::Pipe(ref mut writer) => writer.flush(),
            Self::Target(ref mut writer) => writer.flush(),
        }
    }
}

/// A bound port endpoint: an input or output pipe end.
pub(crate) enum Endpoint<'name, 'writer> {
    Input(InputStream<'name>),
    Output(OutputStream<'name, 'writer>),
}

impl<'name, 'writer> Endpoint<'name, 'writer> {
    /// Single typed endpoint: the length-1 case of the vec conversions.
    pub(crate) fn into_input(self) -> Result<InputStream<'name>> {
        match self {
            Endpoint::Input(input) => Ok(input),
            Endpoint::Output(_) => Err(WizardError::BuildError(
                "expected an input endpoint".to_owned(),
            )),
        }
    }

    /// Single typed endpoint: the length-1 case of the vec conversions.
    pub(crate) fn into_output(self) -> Result<OutputStream<'name, 'writer>> {
        match self {
            Endpoint::Output(output) => Ok(output),
            Endpoint::Input(_) => Err(WizardError::BuildError(
                "expected an output endpoint".to_owned(),
            )),
        }
    }

    /// Converts every endpoint into an input handle.
    pub(crate) fn into_inputs(
        eps: impl IntoIterator<Item = Self>,
    ) -> Result<Vec<InputStream<'name>>> {
        eps.into_iter().map(Self::into_input).collect()
    }

    /// Converts every endpoint into an output handle.
    pub(crate) fn into_outputs(
        eps: impl IntoIterator<Item = Self>,
    ) -> Result<Vec<OutputStream<'name, 'writer>>> {
        eps.into_iter().map(Self::into_output).collect()
    }
}

/// Node-local port endpoints in planner order.
type BoundEndpoints<'name, 'writer> = Vec<(PortId, Endpoint<'name, 'writer>)>;

/// The owned endpoints of one prepared node, addressed by the node module's port constants.
pub(crate) struct NodePorts<'name, 'writer> {
    pub(crate) endpoints: Vec<(PortId, Endpoint<'name, 'writer>)>,
}

impl<'name, 'writer> NodePorts<'name, 'writer> {
    /// Fixed port, always bound.
    pub(crate) fn take(&mut self, port: PortId) -> Result<Endpoint<'name, 'writer>> {
        let index = self
            .endpoints
            .iter()
            .position(|endpoint| endpoint.0 == port)
            .ok_or_else(|| {
                WizardError::BuildError(format!("missing endpoint for port {port:?}"))
            })?;

        Ok(self.endpoints.remove(index).1)
    }

    /// Dynamic ports from `first` onward, in planner order. `None` takes every remaining endpoint.
    pub(crate) fn take_from(
        &mut self,
        first: PortId,
        expected: Option<usize>,
    ) -> Result<Vec<(PortId, Endpoint<'name, 'writer>)>> {
        let (taken, remaining): (
            BoundEndpoints<'name, 'writer>,
            BoundEndpoints<'name, 'writer>,
        ) = core::mem::take(&mut self.endpoints)
            .into_iter()
            .partition(|endpoint| endpoint.0 >= first);
        if let Some(expected) = expected
            && taken.len() != expected
        {
            return Err(WizardError::BuildError(format!(
                "dynamic port count mismatch: {} != {}",
                taken.len(),
                expected,
            )));
        }
        self.endpoints = remaining;

        Ok(taken)
    }
}
