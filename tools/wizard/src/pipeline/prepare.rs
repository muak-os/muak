//! Pipe allocation and generic binding of logical nodes into owned `PreparedNode` values.

use crate::error::{Result, WizardError};
use crate::pipeline::graph::{Graph, Node, StreamId};
use crate::pipeline::preflight::PreflightedGraph;
use crate::pipeline::runtime::{Endpoint, InputStream, NodePorts, OutputStream, PreparedNode};
use crate::stream::pipe::Pipe;

/// The bound nodes plus the preflight data lists the executor keeps alive.
type BoundGraph = (
    Vec<PreparedNode>,
    Vec<mumi::payload::Planned>,
    Vec<(String, u64)>,
);

/// Binds the preflighted graph into a `BoundGraph` with owned pipe endpoints.
pub(crate) fn bind_nodes(preflighted: PreflightedGraph) -> Result<BoundGraph> {
    let PreflightedGraph {
        graph,
        planned_payloads,
        overlay_files,
    } = preflighted;
    let mut ports = allocate(&graph)?;
    let mut nodes = Vec::with_capacity(graph.nodes().len());

    for node in graph.nodes() {
        nodes.push(bind_node(node, &mut ports)?);
    }

    ports.assert_empty()?;

    Ok((nodes, planned_payloads, overlay_files))
}

fn bind_node(node: &Node, ports: &mut PortTable) -> Result<PreparedNode> {
    let mut endpoints = Vec::with_capacity(node.inputs.len().saturating_add(node.outputs.len()));
    for binding in &node.inputs {
        endpoints.push((
            binding.port,
            Endpoint::Input(ports.take_input(binding.stream)?),
        ));
    }
    for binding in &node.outputs {
        endpoints.push((
            binding.port,
            Endpoint::Output(ports.take_output(binding.stream)?),
        ));
    }

    Ok(PreparedNode {
        kind: node.kind,
        ports: NodePorts { endpoints },
    })
}

/// Creates one pipe per stream.
fn allocate(graph: &Graph) -> Result<PortTable> {
    let mut inputs = Vec::with_capacity(graph.streams().len());
    let mut outputs = Vec::with_capacity(graph.streams().len());
    for stream in graph.streams() {
        let (reader, writer) = Pipe::new("stream pipe")?.split();
        inputs.push(Some(InputStream {
            size: stream.size,
            reader,
        }));
        outputs.push(Some(OutputStream {
            size: stream.size,
            writer,
        }));
    }

    Ok(PortTable { inputs, outputs })
}

/// Construction-time ownership ledger for pipe endpoints.
struct PortTable {
    inputs: Vec<Option<InputStream>>,
    outputs: Vec<Option<OutputStream>>,
}

impl PortTable {
    /// Consumes the input endpoint of a stream, once.
    fn take_input(&mut self, stream: StreamId) -> Result<InputStream> {
        self.inputs
            .get_mut(stream.0)
            .and_then(Option::take)
            .ok_or_else(|| {
                WizardError::BuildError(format!("endpoint for stream {stream:?} unavailable"))
            })
    }

    /// Consumes the output endpoint of a stream, once.
    fn take_output(&mut self, stream: StreamId) -> Result<OutputStream> {
        self.outputs
            .get_mut(stream.0)
            .and_then(Option::take)
            .ok_or_else(|| {
                WizardError::BuildError(format!("endpoint for stream {stream:?} unavailable"))
            })
    }

    /// Rejects unconsumed or duplicate endpoints before any node starts.
    fn assert_empty(self) -> Result<()> {
        if let Some(index) = self.inputs.iter().position(Option::is_some) {
            return Err(WizardError::BuildError(format!(
                "unconsumed endpoint for stream {index}"
            )));
        }
        if let Some(index) = self.outputs.iter().position(Option::is_some) {
            return Err(WizardError::BuildError(format!(
                "unconsumed endpoint for stream {index}"
            )));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::Artifact;
    use crate::pipeline::graph::{NodeKind, PortId};

    fn fused_graph() -> Graph {
        // ARRANGE
        let mut graph = Graph::new();
        let producer = graph.add_node(NodeKind::Concat);
        let sink = graph.add_node(NodeKind::ArtifactSink {
            artifact: Artifact::Kernel,
        });
        let stream = graph.add_output(producer, PortId(0)).expect("add output");
        graph.bind_input(sink, PortId(0), stream).expect("bind");

        graph
    }

    #[test]
    fn every_stream_allocates_a_pipe() {
        // ARRANGE
        let graph = fused_graph();

        // ACT
        let table = allocate(&graph).expect("allocate");

        // ASSERT
        let stream = graph.streams().iter().next().expect("stream");
        assert!(table.inputs.get(stream.id.0).expect("slot").is_some());
        assert!(table.outputs.get(stream.id.0).expect("slot").is_some());
    }
}
