//! Runtime stream handles and prepared node values.

use std::io::{self, Write};
use std::os::unix::net::UnixStream;

use crate::error::{Result, WizardError};
use crate::pipeline::graph::PortId;

/// A live input pipe end with the stream's fixed size.
pub(crate) struct InputStream<'a> {
    pub(crate) name: &'a str,
    pub(crate) size: u64,
    pub(crate) reader: UnixStream,
}

/// A live output pipe end with the stream's fixed size.
pub(crate) struct OutputStream<'a> {
    pub(crate) name: &'a str,
    pub(crate) size: u64,
    pub(crate) writer: UnixStream,
}

/// A bound port endpoint: an input or output pipe end.
pub(crate) enum Endpoint<'a> {
    Input(InputStream<'a>),
    Output(OutputStream<'a>),
}

impl<'a> Endpoint<'a> {
    /// Single typed endpoint: the length-1 case of the vec conversions.
    pub(crate) fn into_input(self) -> Result<InputStream<'a>> {
        match self {
            Endpoint::Input(input) => Ok(input),
            Endpoint::Output(_) => Err(WizardError::BuildError(
                "expected an input endpoint".to_owned(),
            )),
        }
    }

    /// Single typed endpoint: the length-1 case of the vec conversions.
    pub(crate) fn into_output(self) -> Result<OutputStream<'a>> {
        match self {
            Endpoint::Output(output) => Ok(output),
            Endpoint::Input(_) => Err(WizardError::BuildError(
                "expected an output endpoint".to_owned(),
            )),
        }
    }

    /// Converts every endpoint into an input handle.
    pub(crate) fn into_inputs(eps: impl IntoIterator<Item = Self>) -> Result<Vec<InputStream<'a>>> {
        eps.into_iter().map(Self::into_input).collect()
    }

    /// Converts every endpoint into an output handle.
    pub(crate) fn into_outputs(
        eps: impl IntoIterator<Item = Self>,
    ) -> Result<Vec<OutputStream<'a>>> {
        eps.into_iter().map(Self::into_output).collect()
    }
}

/// The owned endpoints of one prepared node, addressed by the node module's port constants.
pub(crate) struct NodePorts<'a> {
    pub(crate) endpoints: Vec<(PortId, Endpoint<'a>)>,
}

impl<'a> NodePorts<'a> {
    /// Fixed port, always bound.
    pub(crate) fn take(&mut self, port: PortId) -> Result<Endpoint<'a>> {
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
    ) -> Result<Vec<(PortId, Endpoint<'a>)>> {
        let (taken, remaining) = split_endpoints(core::mem::take(&mut self.endpoints), first);
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

/// Write adapter over a `&mut (dyn Write + Send)` for generic `W: Write` tool APIs.
pub(crate) struct DynWriter<'a>(&'a mut (dyn Write + Send));

impl<'a> DynWriter<'a> {
    pub(crate) fn new(writer: &'a mut (dyn Write + Send)) -> Self {
        Self(writer)
    }
}

impl Write for DynWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// Node-local port endpoints in planner order.
type BoundEndpoints<'a> = Vec<(PortId, Endpoint<'a>)>;

fn split_endpoints(
    endpoints: BoundEndpoints<'_>,
    first: PortId,
) -> (BoundEndpoints<'_>, BoundEndpoints<'_>) {
    endpoints
        .into_iter()
        .partition(|endpoint| endpoint.0 >= first)
}
