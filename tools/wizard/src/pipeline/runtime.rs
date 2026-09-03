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
    /// Every chunk written to all sinks, preserving backpressure.
    Fanout(Vec<OutputWriter<'writer>>),
}

impl<'writer> OutputWriter<'writer> {
    fn write_all_sinks(sinks: &mut [OutputWriter<'writer>], buf: &[u8]) -> io::Result<usize> {
        for sink in sinks {
            sink.write_all(buf)?;
        }

        Ok(buf.len())
    }

    fn flush_sinks(sinks: &mut [OutputWriter<'writer>]) -> io::Result<()> {
        for sink in sinks {
            sink.flush()?;
        }

        Ok(())
    }
}

impl Write for OutputWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match *self {
            Self::Fanout(ref mut sinks) => Self::write_all_sinks(sinks, buf),
            Self::Pipe(ref mut writer) => writer.write(buf),
            Self::Target(ref mut writer) => writer.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match *self {
            Self::Fanout(ref mut sinks) => Self::flush_sinks(sinks),
            Self::Pipe(ref mut writer) => writer.flush(),
            Self::Target(ref mut writer) => writer.flush(),
        }
    }
}

/// The owned endpoints of one prepared node, addressed by the node module's port constants.
pub(crate) struct NodePorts<'name, 'writer> {
    inputs: Vec<(PortId, InputStream<'name>)>,
    outputs: Vec<(PortId, OutputStream<'name, 'writer>)>,
}

impl<'name, 'writer> NodePorts<'name, 'writer> {
    /// Builds the node's endpoints in planner binding order.
    pub(crate) fn new(
        inputs: Vec<(PortId, InputStream<'name>)>,
        outputs: Vec<(PortId, OutputStream<'name, 'writer>)>,
    ) -> Self {
        Self { inputs, outputs }
    }

    /// Takes the input handle bound to `port`.
    pub(crate) fn input(&mut self, port: PortId) -> Result<InputStream<'name>> {
        Self::take(&mut self.inputs, port)
    }

    /// Takes the output handle bound to `port`.
    pub(crate) fn output(&mut self, port: PortId) -> Result<OutputStream<'name, 'writer>> {
        Self::take(&mut self.outputs, port)
    }

    /// Takes the dynamic input ports from `first` onward, in planner order.
    pub(crate) fn inputs_from(
        &mut self,
        first: PortId,
        expected: Option<usize>,
    ) -> Result<Vec<InputStream<'name>>> {
        Self::take_from(&mut self.inputs, first, expected)
            .map(|endpoints| endpoints.into_iter().map(|(_, input)| input).collect())
    }

    /// Takes the dynamic output ports from `first` onward, in planner order.
    pub(crate) fn outputs_from(
        &mut self,
        first: PortId,
        expected: Option<usize>,
    ) -> Result<Vec<OutputStream<'name, 'writer>>> {
        Self::take_from(&mut self.outputs, first, expected)
            .map(|endpoints| endpoints.into_iter().map(|(_, output)| output).collect())
    }

    /// Takes the dynamic output ports from `first` onward, paired with their port.
    pub(crate) fn output_pairs_from(
        &mut self,
        first: PortId,
        expected: Option<usize>,
    ) -> Result<Vec<(PortId, OutputStream<'name, 'writer>)>> {
        Self::take_from(&mut self.outputs, first, expected)
    }

    fn take<T>(endpoints: &mut Vec<(PortId, T)>, port: PortId) -> Result<T> {
        let index = endpoints
            .iter()
            .position(|endpoint| endpoint.0 == port)
            .ok_or_else(|| {
                WizardError::BuildError(format!("missing endpoint for port {port:?}"))
            })?;

        Ok(endpoints.remove(index).1)
    }

    fn take_from<T>(
        endpoints: &mut Vec<(PortId, T)>,
        first: PortId,
        expected: Option<usize>,
    ) -> Result<Vec<(PortId, T)>> {
        let (taken, remaining): (Vec<_>, Vec<_>) = endpoints
            .drain(..)
            .partition(|endpoint| endpoint.0 >= first);
        *endpoints = remaining;
        if let Some(expected) = expected
            && taken.len() != expected
        {
            return Err(WizardError::BuildError(format!(
                "dynamic port count mismatch: {} != {}",
                taken.len(),
                expected,
            )));
        }

        Ok(taken)
    }
}
