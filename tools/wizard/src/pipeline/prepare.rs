//! Pipe allocation and generic binding of logical nodes into owned `PreparedNode` values.

use std::io::Write;

use crate::artifact::Artifact;
use crate::error::{Result, WizardError};
use crate::pipeline::graph::{Graph, Node, NodeKind, StreamId};
use crate::pipeline::preflight::PreflightedGraph;
use crate::pipeline::runtime::{
    Endpoint, InputStream, NodePorts, OutputStream, PreparedNode, PreparedSink, PreparedTask,
};
use crate::stream::pipe::Pipe;

/// User artifact writers, consumed once each at bind time.
pub(crate) struct TargetWriters<'a> {
    slots: [Option<&'a mut (dyn Write + Send)>; Artifact::COUNT],
}

impl<'a> TargetWriters<'a> {
    /// Builds the slot array from the request's target pairs.
    #[must_use]
    pub(crate) fn new(targets: Vec<(Artifact, &'a mut (dyn Write + Send))>) -> Self {
        let mut slots: [Option<&'a mut (dyn Write + Send)>; Artifact::COUNT] =
            [const { None }; Artifact::COUNT];
        fill_slots(&mut slots, targets);

        Self { slots }
    }

    /// Takes the writer for an artifact, if still available.
    pub(crate) fn take(&mut self, artifact: Artifact) -> Option<&'a mut (dyn Write + Send)> {
        self.slots
            .get_mut(artifact.to_index())
            .and_then(Option::take)
    }

    fn assert_empty(&self) -> Result<()> {
        if let Some(index) = self.slots.iter().position(Option::is_some) {
            return Err(WizardError::BuildError(format!(
                "unconsumed target writer for artifact {index}"
            )));
        }

        Ok(())
    }
}

/// The bound tasks plus the preflight data lists the executor keeps alive.
type BoundGraph<'a> = (
    Vec<PreparedTask<'a>>,
    Vec<mumi::payload::Planned>,
    Vec<(String, u64)>,
);

pub(crate) fn bind_nodes<'a>(
    preflighted: PreflightedGraph,
    targets: &mut TargetWriters<'a>,
) -> Result<BoundGraph<'a>> {
    let PreflightedGraph {
        graph,
        planned_payloads,
        overlay_files,
    } = preflighted;
    let mut ports = allocate(&graph)?;
    let mut tasks = Vec::with_capacity(graph.nodes().len());

    for node in graph.nodes() {
        let task = if let NodeKind::ArtifactSink { .. } = node.kind {
            PreparedTask::Sink(bind_sink(node, &mut ports, targets)?)
        } else {
            PreparedTask::Node(bind_node(node, &mut ports)?)
        };
        tasks.push(task);
    }

    ports.assert_empty()?;
    targets.assert_empty()?;

    Ok((tasks, planned_payloads, overlay_files))
}

fn fill_slots<'a>(
    slots: &mut [Option<&'a mut (dyn Write + Send)>],
    targets: Vec<(Artifact, &'a mut (dyn Write + Send))>,
) {
    for (artifact, writer) in targets {
        if let Some(slot) = slots.get_mut(artifact.to_index()) {
            *slot = Some(writer);
        }
    }
}

/// Binds a non-sink node's endpoints from the pipe table.
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

/// Binds an `ArtifactSink` node to its input pipe and its user writer.
fn bind_sink<'a>(
    node: &Node,
    ports: &mut PortTable,
    targets: &mut TargetWriters<'a>,
) -> Result<PreparedSink<'a>> {
    let NodeKind::ArtifactSink { artifact } = node.kind else {
        return Err(WizardError::BuildError(
            "sink node has no artifact".to_owned(),
        ));
    };
    let binding = node
        .input_bindings()
        .next()
        .ok_or_else(|| WizardError::BuildError("sink node has no input".to_owned()))?;
    let input = ports.take_input(binding.stream)?;
    let writer = targets
        .take(artifact)
        .ok_or_else(|| WizardError::BuildError(format!("missing target writer for {artifact}")))?;

    Ok(PreparedSink { input, writer })
}

/// Construction-time ownership ledger for pipe endpoints.
struct PortTable {
    inputs: Vec<Option<InputStream>>,
    outputs: Vec<Option<OutputStream>>,
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
    use crate::pipeline::graph::PortId;

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
