//! Reads the input stream once and writes every chunk to all outputs.

use std::io::Write;

use crate::error::{Result, WizardError};
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::{Graph, NodeId, PortId};
use crate::pipeline::runtime::{Endpoint, NodePorts};
use crate::stream;

pub(crate) const FANOUT_INPUT: PortId = PortId(0);
pub(crate) const FANOUT_OUTPUTS_FIRST: PortId = PortId(1);

/// Every fanout output copies the input stream's size and name.
pub(crate) fn preflight(graph: &mut Graph, id: NodeId) -> Result<()> {
    let input = graph.node(id)?.input(FANOUT_INPUT)?;
    let source = graph.stream(input)?;
    let size = source.size;
    let name = source.name.clone();
    let bindings = graph
        .node(id)?
        .output_bindings()
        .copied()
        .collect::<Vec<_>>();
    for binding in bindings {
        let stream = graph.stream_mut(binding.stream)?;
        stream.size = size;
        stream.name.clone_from(&name);
    }

    Ok(())
}

pub(crate) fn run(ports: &mut NodePorts<'_>) -> Result<NodeReport> {
    let mut input = ports.take(FANOUT_INPUT)?.into_input()?;
    let mut outputs = Endpoint::into_outputs(
        ports
            .take_from(FANOUT_OUTPUTS_FIRST, None)?
            .into_iter()
            .map(|(_, endpoint)| endpoint),
    )?;
    let mut sinks: Vec<&mut (dyn Write + Send)> = outputs
        .iter_mut()
        .map(|output| -> &mut (dyn Write + Send) { &mut output.writer })
        .collect();

    stream::fanout::copy_to_all(&mut input.reader, &mut sinks)
        .map_err(|e| WizardError::BuildError(format!("fanout stream: {e}")))?;

    Ok(NodeReport::Empty)
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;
    use std::os::unix::net::UnixStream;

    use super::*;
    use crate::pipeline::graph::PortId;
    use crate::pipeline::runtime::{Endpoint, InputStream, NodePorts, OutputStream};

    #[test]
    fn run_fans_out_bytes_to_all_outputs() {
        // ARRANGE
        let (mut input_writer, input_reader) = UnixStream::pair().expect("input pipe");
        let (left_writer, mut left_reader) = UnixStream::pair().expect("output pipe");
        let (right_writer, mut right_reader) = UnixStream::pair().expect("output pipe");
        input_writer.write_all(b"fanned").expect("write input");
        drop(input_writer);

        let mut ports = NodePorts {
            endpoints: vec![
                (
                    FANOUT_INPUT,
                    Endpoint::Input(InputStream {
                        size: 6,
                        name: "fanned",
                        reader: input_reader,
                    }),
                ),
                (
                    PortId(1),
                    Endpoint::Output(OutputStream {
                        size: 6,
                        name: "left",
                        writer: left_writer,
                    }),
                ),
                (
                    PortId(2),
                    Endpoint::Output(OutputStream {
                        size: 6,
                        name: "right",
                        writer: right_writer,
                    }),
                ),
            ],
        };

        // ACT
        run(&mut ports).expect("fanout run");

        // ASSERT
        let mut first = Vec::new();
        left_reader.read_to_end(&mut first).expect("read first");
        let mut second = Vec::new();
        right_reader.read_to_end(&mut second).expect("read second");
        assert_eq!(first, b"fanned");
        assert_eq!(second, b"fanned");
    }
}
