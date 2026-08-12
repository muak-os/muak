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

/// One output port at runtime: a live pipe, or a fused user writer.
pub(crate) enum OutputSink<'a> {
    Pipe(OutputStream),
    Writer {
        size: u64,
        writer: &'a mut (dyn Write + Send),
    },
}

impl OutputSink<'_> {
    /// The stream's fixed size.
    #[must_use]
    pub(crate) fn size(&self) -> u64 {
        match *self {
            OutputSink::Pipe(ref pipe) => pipe.size,
            OutputSink::Writer { size, .. } => size,
        }
    }

    /// The underlying writer, identical for piped and fused outputs.
    pub(crate) fn writer(&mut self) -> &mut (dyn Write + Send) {
        match *self {
            OutputSink::Pipe(ref mut pipe) => &mut pipe.writer,
            OutputSink::Writer { ref mut writer, .. } => writer,
        }
    }
}

/// A bound port endpoint: an input handle or an output sink.
pub(crate) enum Endpoint<'a> {
    Input(InputStream),
    Output(OutputSink<'a>),
}

impl<'a> Endpoint<'a> {
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
    pub(crate) fn into_output(self) -> Result<OutputSink<'a>> {
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

    /// Converts every endpoint into an output sink.
    pub(crate) fn into_outputs(eps: impl IntoIterator<Item = Self>) -> Result<Vec<OutputSink<'a>>> {
        eps.into_iter().map(Self::into_output).collect()
    }
}

/// The owned endpoints of one prepared node, addressed by the node module's
/// port constants.
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

    /// Dynamic ports from `first` onward, in planner order. `Some(n)` checks
    /// the count against the paired preflight data list; `None` takes every
    /// remaining endpoint.
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

/// A bound, owned node ready to run on its own scoped thread.
pub(crate) struct PreparedNode<'a> {
    pub(crate) kind: NodeKind,
    pub(crate) ports: NodePorts<'a>,
    /// Requested artifact writer; `Some` only for the Iso/Raw media nodes.
    pub(crate) target: Option<&'a mut (dyn Write + Send)>,
}

/// Node-local port endpoints in planner order.
type BoundEndpoints<'a> = Vec<(PortId, Endpoint<'a>)>;

/// Splits endpoints into those at or after `first` and the rest.
fn split_endpoints(
    endpoints: BoundEndpoints<'_>,
    first: PortId,
) -> (BoundEndpoints<'_>, BoundEndpoints<'_>) {
    endpoints
        .into_iter()
        .partition(|endpoint| endpoint.0 >= first)
}
