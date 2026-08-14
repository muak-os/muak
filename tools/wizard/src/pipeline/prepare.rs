//! Pipe allocation and generic binding of logical nodes into owned `PreparedNode` values.

use crate::error::{Result, WizardError};
use crate::pipeline::graph::{Graph, Node, NodeKind, StreamId};
use crate::pipeline::runtime::{Endpoint, InputStream, NodePorts, OutputStream};
use crate::stream::pipe::Pipe;

/// Binds the preflighted graph into owned `PreparedNode` values with pipe endpoints.
pub(crate) fn bind_nodes(graph: &Graph) -> Result<Vec<PreparedNode<'_>>> {
    let mut ports = allocate(graph)?;
    let mut nodes = Vec::with_capacity(graph.nodes().len());

    for node in graph.nodes() {
        nodes.push(bind_node(node, &mut ports)?);
    }

    ports.assert_empty()?;

    Ok(nodes)
}

fn bind_node<'a>(node: &Node, ports: &mut PortTable<'a>) -> Result<PreparedNode<'a>> {
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
fn allocate(graph: &Graph) -> Result<PortTable<'_>> {
    let mut inputs = Vec::with_capacity(graph.streams().len());
    let mut outputs = Vec::with_capacity(graph.streams().len());
    for stream in graph.streams() {
        let (reader, writer) = Pipe::new("stream pipe")?.split();
        inputs.push(Some(InputStream {
            size: stream.size,
            name: &stream.name,
            reader,
        }));
        outputs.push(Some(OutputStream {
            size: stream.size,
            name: &stream.name,
            writer,
        }));
    }

    Ok(PortTable { inputs, outputs })
}

/// Construction-time ownership ledger for pipe endpoints.
struct PortTable<'a> {
    inputs: Vec<Option<InputStream<'a>>>,
    outputs: Vec<Option<OutputStream<'a>>>,
}

impl<'a> PortTable<'a> {
    /// Consumes the input endpoint of a stream, once.
    fn take_input(&mut self, stream: StreamId) -> Result<InputStream<'a>> {
        self.inputs
            .get_mut(stream.0)
            .and_then(Option::take)
            .ok_or_else(|| {
                WizardError::BuildError(format!("endpoint for stream {stream:?} unavailable"))
            })
    }

    /// Consumes the output endpoint of a stream, once.
    fn take_output(&mut self, stream: StreamId) -> Result<OutputStream<'a>> {
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

/// A bound, owned node ready to run on its own scoped thread.
pub(crate) struct PreparedNode<'a> {
    pub(crate) kind: NodeKind,
    pub(crate) ports: NodePorts<'a>,
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

    #[test]
    fn stream_names_reach_endpoints() {
        // ARRANGE
        let mut graph = fused_graph();
        let stream_id = graph.streams().iter().next().expect("stream").id;
        graph.stream_mut(stream_id).expect("stream").name = "kernel".to_owned();

        // ACT
        let table = allocate(&graph).expect("allocate");

        // ASSERT
        assert_eq!(
            table
                .inputs
                .get(stream_id.0)
                .expect("slot")
                .as_ref()
                .expect("input")
                .name,
            "kernel"
        );
        assert_eq!(
            table
                .outputs
                .get(stream_id.0)
                .expect("slot")
                .as_ref()
                .expect("output")
                .name,
            "kernel"
        );
    }
}
