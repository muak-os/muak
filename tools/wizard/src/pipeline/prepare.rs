//! Pipe allocation and generic binding of logical nodes into owned `PreparedNode` values.

use std::io::Write;
use std::os::unix::net::UnixStream;

use crate::artifact::Artifact;
use crate::error::{Result, WizardError};
use crate::nodes::NodeKind;
use crate::pipeline::context::TargetWriters;
use crate::pipeline::graph::Graph;
use crate::pipeline::node::{Node, StreamId};
use crate::pipeline::runtime::{Endpoint, InputStream, NodePorts, OutputStream, OutputWriter};
use crate::stream::pipe::Pipe;

/// A bound, owned node ready to run on its own scoped thread.
pub(crate) struct PreparedNode<'name, 'writer> {
    pub(crate) kind: NodeKind,
    pub(crate) ports: NodePorts<'name, 'writer>,
}

/// Binds the preflighted graph into owned `PreparedNode` values with pipe
/// endpoints, fusing terminal artifact streams into the user's writers.
pub(crate) fn bind_nodes<'name, 'writer>(
    graph: &'name Graph,
    writers: &mut TargetWriters<'writer>,
) -> Result<Vec<PreparedNode<'name, 'writer>>> {
    let mut ports = allocate(graph, writers)?;
    let mut nodes = Vec::with_capacity(graph.nodes().len());

    for node in graph.nodes() {
        nodes.push(bind_node(node, &mut ports)?);
    }

    ports.assert_empty()?;

    Ok(nodes)
}

fn bind_node<'name, 'writer>(
    node: &Node,
    ports: &mut PortTable<'name, 'writer>,
) -> Result<PreparedNode<'name, 'writer>> {
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

/// Creates one pipe per intermediate stream and one fused writer per terminal stream.
fn allocate<'name, 'writer>(
    graph: &'name Graph,
    writers: &mut TargetWriters<'writer>,
) -> Result<PortTable<'name, 'writer>> {
    let mut table = PortTable::with_capacity(graph.streams().len());
    for stream in graph.streams() {
        let name = &stream.name;
        if let Some(artifact) = stream.artifact {
            let writer = writers
                .take(artifact)
                .ok_or_else(|| missing_writer(artifact))?;
            table.push_target(name, stream.size, writer);
        } else {
            let (reader, writer) = Pipe::new("stream pipe")?.split();
            table.push_pipe(name, stream.size, reader, writer);
        }
    }

    Ok(table)
}

fn missing_writer(artifact: Artifact) -> WizardError {
    WizardError::BuildError(format!("missing target writer for {artifact}"))
}

/// Construction-time ownership ledger for pipe and fused endpoints.
struct PortTable<'name, 'writer> {
    inputs: Vec<Option<InputStream<'name>>>,
    outputs: Vec<Option<OutputStream<'name, 'writer>>>,
}

impl<'name, 'writer> PortTable<'name, 'writer> {
    fn with_capacity(streams: usize) -> Self {
        Self {
            inputs: Vec::with_capacity(streams),
            outputs: Vec::with_capacity(streams),
        }
    }

    /// Records one piped stream: a readable input and a writable output.
    fn push_pipe(&mut self, name: &'name str, size: u64, reader: UnixStream, writer: UnixStream) {
        self.inputs.push(Some(InputStream { name, size, reader }));
        self.outputs.push(Some(OutputStream {
            name,
            size,
            writer: OutputWriter::Pipe(writer),
        }));
    }

    /// Records one fused terminal stream: no input, the user writer as output.
    fn push_target(
        &mut self,
        name: &'name str,
        size: u64,
        writer: &'writer mut (dyn Write + Send),
    ) {
        self.inputs.push(None);
        self.outputs.push(Some(OutputStream {
            name,
            size,
            writer: OutputWriter::Target(writer),
        }));
    }

    /// Consumes the input endpoint of a stream, once.
    fn take_input(&mut self, stream: StreamId) -> Result<InputStream<'name>> {
        self.inputs
            .get_mut(stream.0)
            .and_then(Option::take)
            .ok_or_else(|| {
                WizardError::BuildError(format!("endpoint for stream {stream:?} unavailable"))
            })
    }

    /// Consumes the output endpoint of a stream, once.
    fn take_output(&mut self, stream: StreamId) -> Result<OutputStream<'name, 'writer>> {
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
    use std::os::unix::net::UnixStream;

    use super::*;
    use crate::artifact::Artifact;
    use crate::pipeline::context::TargetWriters;
    use crate::pipeline::node::PortId;

    fn piped_graph() -> Graph {
        // ARRANGE
        let mut graph = Graph::new();
        let producer = graph.add_node(NodeKind::Concat);
        let consumer = graph.add_node(NodeKind::Uki);
        let stream = graph.add_output(producer, PortId(0)).expect("add output");
        graph.bind_input(consumer, PortId(0), stream).expect("bind");

        graph
    }

    fn fused_graph() -> Graph {
        // ARRANGE
        let mut graph = Graph::new();
        let producer = graph.add_node(NodeKind::KernelPull);
        let stream = graph.add_output(producer, PortId(0)).expect("add output");
        graph.stream_mut(stream).expect("stream").artifact = Some(Artifact::Kernel);

        graph
    }

    #[test]
    fn every_intermediate_stream_allocates_a_pipe() {
        // ARRANGE
        let graph = piped_graph();
        let mut writers = TargetWriters::new(Vec::new());

        // ACT
        let table = allocate(&graph, &mut writers).expect("allocate");

        // ASSERT
        let stream = graph.streams().iter().next().expect("stream");
        assert!(table.inputs.get(stream.id.0).expect("slot").is_some());
        assert!(table.outputs.get(stream.id.0).expect("slot").is_some());
    }

    #[test]
    fn terminal_streams_fuse_into_the_target_writer() {
        // ARRANGE
        let graph = fused_graph();
        let mut writer = Vec::new();
        let mut writers = TargetWriters::new(vec![(Artifact::Kernel, &mut writer)]);

        // ACT
        let mut table = allocate(&graph, &mut writers).expect("allocate");

        // ASSERT
        let stream = graph.streams().iter().next().expect("stream");
        assert!(
            table.inputs.get(stream.id.0).expect("slot").is_none(),
            "a fused stream must allocate no pipe input"
        );
        let output = table
            .outputs
            .get_mut(stream.id.0)
            .expect("slot")
            .take()
            .expect("output");
        match output.writer {
            OutputWriter::Target(_) => {}
            OutputWriter::Pipe(_) => panic!("terminal stream must hold the target writer"),
        }
        drop(output);
        drop(writer);
    }

    #[test]
    fn fused_binding_runs_bytes_through_the_target_writer() {
        // ARRANGE
        let graph = fused_graph();
        let mut writer = Vec::new();
        let mut writers = TargetWriters::new(vec![(Artifact::Kernel, &mut writer)]);
        let (mut pipe_writer, mut pipe_reader) = UnixStream::pair().expect("pipe");

        // ACT
        let mut table = allocate(&graph, &mut writers).expect("allocate");
        let mut output = table
            .take_output(graph.streams().iter().next().expect("stream").id)
            .expect("output");
        pipe_writer.write_all(b"artifact bytes").expect("write");
        drop(pipe_writer);
        std::io::copy(&mut pipe_reader, &mut output.writer).expect("copy");
        drop(output);

        // ASSERT
        assert_eq!(writer, b"artifact bytes");
    }

    #[test]
    fn stream_names_reach_endpoints() {
        // ARRANGE
        let mut graph = fused_graph();
        let stream_id = graph.streams().iter().next().expect("stream").id;
        graph.stream_mut(stream_id).expect("stream").name = "kernel".to_owned();
        let mut writer = Vec::new();
        let mut writers = TargetWriters::new(vec![(Artifact::Kernel, &mut writer)]);

        // ACT
        let table = allocate(&graph, &mut writers).expect("allocate");

        // ASSERT
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
