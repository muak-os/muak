//! Pipe allocation and generic binding of logical nodes into owned `PreparedNode` values.

use std::io::Write;

use crate::artifact::Artifact;
use crate::error::{Result, WizardError};
use crate::pipeline::graph::{Graph, NodeKind, PortBinding, StreamId};
use crate::pipeline::preflight::PreflightedGraph;
use crate::pipeline::runtime::{
    Endpoint, InputStream, NodePorts, OutputSink, OutputStream, PreparedNode,
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
}

/// Binds every logical node generically into a `PreparedNode` value.
type BoundGraph<'a> = (
    Vec<PreparedNode<'a>>,
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
    let mut nodes = Vec::with_capacity(graph.nodes().len());

    for node in graph.nodes() {
        if matches!(&node.kind, NodeKind::ArtifactSink { .. }) {
            continue;
        }

        let mut endpoints =
            Vec::with_capacity(node.inputs.len().saturating_add(node.outputs.len()));
        for binding in &node.inputs {
            endpoints.push((
                binding.port,
                Endpoint::Input(ports.take_input(binding.stream)?),
            ));
        }
        for binding in &node.outputs {
            let sink = fuse_or_pipe(&graph, binding, &mut ports, targets)?;
            endpoints.push((binding.port, Endpoint::Output(sink)));
        }
        let target = media_target(node.kind, targets)?;

        nodes.push(PreparedNode {
            kind: node.kind,
            ports: NodePorts { endpoints },
            target,
        });
    }

    ports.assert_empty()?;

    Ok((nodes, planned_payloads, overlay_files))
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

/// Construction-time ownership ledger for pipe endpoints.
struct PortTable {
    inputs: Vec<Option<InputStream>>,
    outputs: Vec<Option<OutputStream>>,
}

/// Creates one pipe per non-fused stream.
fn allocate(graph: &Graph) -> Result<PortTable> {
    let mut inputs = Vec::with_capacity(graph.streams().len());
    let mut outputs = Vec::with_capacity(graph.streams().len());
    for stream in graph.streams() {
        if is_fused(graph, stream.id) {
            inputs.push(None);
            outputs.push(None);
        } else {
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

/// True when every consumer of the stream is an `ArtifactSink` node.
fn is_fused(graph: &Graph, stream: StreamId) -> bool {
    graph.stream(stream).is_ok_and(|stream| {
        stream.consumers.iter().all(|consumer| {
            graph
                .node(*consumer)
                .is_ok_and(|node| matches!(&node.kind, NodeKind::ArtifactSink { .. }))
        })
    })
}

/// The user writer for a media node, if it is an Iso/Raw node.
fn media_target<'a>(
    kind: NodeKind,
    targets: &mut TargetWriters<'a>,
) -> Result<Option<&'a mut (dyn Write + Send)>> {
    if kind == NodeKind::Iso {
        targets
            .take(Artifact::Iso)
            .map(Some)
            .ok_or_else(|| WizardError::BuildError("missing target writer for iso".to_owned()))
    } else if kind == NodeKind::Raw {
        targets
            .take(Artifact::Raw)
            .map(Some)
            .ok_or_else(|| WizardError::BuildError("missing target writer for raw".to_owned()))
    } else {
        Ok(None)
    }
}

/// A stream whose consumers are all sinks is fused: no pipe is allocated and
/// the user writer is bound directly on the producer.
fn fuse_or_pipe<'a>(
    graph: &Graph,
    binding: &PortBinding,
    ports: &mut PortTable,
    targets: &mut TargetWriters<'a>,
) -> Result<OutputSink<'a>> {
    let stream = graph.stream(binding.stream)?;
    if !is_fused(graph, stream.id) {
        return Ok(OutputSink::Pipe(ports.take_output(binding.stream)?));
    }

    let consumer = *stream
        .consumers
        .first()
        .ok_or_else(|| WizardError::BuildError("fused stream has no consumer".to_owned()))?;
    let NodeKind::ArtifactSink { artifact } = graph.node(consumer)?.kind else {
        return Err(WizardError::BuildError(
            "fused stream sink mismatch".to_owned(),
        ));
    };
    let writer = targets
        .take(artifact)
        .ok_or_else(|| WizardError::BuildError(format!("missing target writer for {artifact}")))?;

    Ok(OutputSink::Writer {
        size: stream.size,
        writer,
    })
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
    fn fused_stream_allocates_no_pipe() {
        // ARRANGE
        let graph = fused_graph();

        // ACT
        let table = allocate(&graph).expect("allocate");

        // ASSERT
        let stream = graph.streams().iter().next().expect("stream");
        assert!(table.inputs.get(stream.id.0).expect("slot").is_none());
        assert!(table.outputs.get(stream.id.0).expect("slot").is_none());
    }
}
