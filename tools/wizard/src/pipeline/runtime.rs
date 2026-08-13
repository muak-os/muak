//! Runtime stream handles and prepared node values.

use std::io::{self, Write};
use std::os::unix::net::UnixStream;

use crate::error::{Result, WizardError};
use crate::pipeline::graph::{NodeKind, PortId};

/// Write adapter over a `&mut (dyn Write + Send)` for generic `W: Write`
/// tool APIs.
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

/// A live input pipe end with the stream's fixed size.
pub(crate) struct InputStream {
    pub(crate) size: u64,
    pub(crate) reader: UnixStream,
}

/// A live output pipe end with the stream's fixed size.
pub(crate) struct OutputStream {
    pub(crate) size: u64,
    pub(crate) writer: UnixStream,
}

/// A bound port endpoint: an input or output pipe end.
pub(crate) enum Endpoint {
    Input(InputStream),
    Output(OutputStream),
}

impl Endpoint {
    /// Single typed endpoint: the length-1 case of the vec conversions.
    pub(crate) fn into_input(self) -> Result<InputStream> {
        match self {
            Endpoint::Input(input) => Ok(input),
            Endpoint::Output(_) => Err(WizardError::BuildError(
                "expected an input endpoint".to_owned(),
            )),
        }
    }

    /// Single typed endpoint: the length-1 case of the vec conversions.
    pub(crate) fn into_output(self) -> Result<OutputStream> {
        match self {
            Endpoint::Output(output) => Ok(output),
            Endpoint::Input(_) => Err(WizardError::BuildError(
                "expected an output endpoint".to_owned(),
            )),
        }
    }

    /// Converts every endpoint into an input handle.
    pub(crate) fn into_inputs(eps: impl IntoIterator<Item = Self>) -> Result<Vec<InputStream>> {
        eps.into_iter().map(Self::into_input).collect()
    }

    /// Converts every endpoint into an output handle.
    pub(crate) fn into_outputs(eps: impl IntoIterator<Item = Self>) -> Result<Vec<OutputStream>> {
        eps.into_iter().map(Self::into_output).collect()
    }
}

/// The owned endpoints of one prepared node, addressed by the node module's port constants.
pub(crate) struct NodePorts {
    pub(crate) endpoints: Vec<(PortId, Endpoint)>,
}

impl NodePorts {
    /// Fixed port, always bound.
    pub(crate) fn take(&mut self, port: PortId) -> Result<Endpoint> {
        let index = self
            .endpoints
            .iter()
            .position(|endpoint| endpoint.0 == port)
            .ok_or_else(|| {
                WizardError::BuildError(format!("missing endpoint for port {port:?}"))
            })?;

        Ok(self.endpoints.remove(index).1)
    }

    /// Dynamic ports from `first` onward, in planner order. `Some(n)` checks
    /// the count against the paired preflight data list; `None` takes every
    /// remaining endpoint.
    pub(crate) fn take_from(
        &mut self,
        first: PortId,
        expected: Option<usize>,
    ) -> Result<Vec<(PortId, Endpoint)>> {
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

/// A bound, owned node ready to run on its own scoped thread.
pub(crate) struct PreparedNode {
    pub(crate) kind: NodeKind,
    pub(crate) ports: NodePorts,
}

/// Node-local port endpoints in planner order.
type BoundEndpoints = Vec<(PortId, Endpoint)>;

/// Splits endpoints into those at or after `first` and the rest.
fn split_endpoints(endpoints: BoundEndpoints, first: PortId) -> (BoundEndpoints, BoundEndpoints) {
    endpoints
        .into_iter()
        .partition(|endpoint| endpoint.0 >= first)
}
