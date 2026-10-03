//! Pipe allocation and generic binding of logical nodes into owned `PreparedNode` values.

use crate::artifact::Artifact;
use crate::error::{Result, WizardError};
use crate::nodes::NodeKind;
use crate::pipeline::context::TargetWriters;
use crate::pipeline::graph::Graph;
use crate::pipeline::node::{Node, StreamId};
use crate::pipeline::runtime::{InputStream, NodePorts, OutputStream, OutputWriter};
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
    let mut inputs = Vec::with_capacity(node.inputs.len());
    let mut outputs = Vec::with_capacity(node.outputs.len());
    for binding in &node.inputs {
        inputs.push((binding.port, ports.take_input(binding.stream)?));
    }
    for binding in &node.outputs {
        outputs.push((binding.port, ports.take_output(binding.stream)?));
    }

    Ok(PreparedNode {
        kind: node.kind,
        ports: NodePorts::new(inputs, outputs),
    })
}

/// Creates one pipe per intermediate stream destination and one fused writer per terminal stream.
fn allocate<'name, 'writer>(
    graph: &'name Graph,
    writers: &mut TargetWriters<'writer>,
) -> Result<PortTable<'name, 'writer>> {
    let mut table = PortTable::with_capacity(graph.streams().len());
    for stream in graph.streams() {
        let name = &stream.name;
        let mut sinks = Vec::new();
        let mut readers = Vec::new();
        for _ in &stream.consumers {
            let (reader, writer) = Pipe::new("stream pipe")?.split();
            sinks.push(OutputWriter::Pipe(writer));
            readers.push(Some(InputStream {
                name,
                size: stream.size,
                reader,
            }));
        }
        if let Some(artifact) = stream.artifact {
            let writer = writers
                .take(artifact)
                .ok_or_else(|| missing_writer(artifact))?;
            sinks.push(OutputWriter::Target(writer));
        }
        let writer = match sinks.pop() {
            Some(single) if sinks.is_empty() => single,
            Some(last) => {
                sinks.push(last);
                OutputWriter::Fanout(sinks)
            }
            None => {
                return Err(WizardError::BuildError(format!(
                    "stream {name} has no destination"
                )));
            }
        };
        table.push_stream(
            readers,
            OutputStream {
                name,
                size: stream.size,
                writer,
            },
        );
    }

    Ok(table)
}

fn missing_writer(artifact: Artifact) -> WizardError {
    WizardError::BuildError(format!("missing target writer for {artifact}"))
}

/// Construction-time ownership ledger for pipe and fused endpoints.
struct PortTable<'name, 'writer> {
    inputs: Vec<Vec<Option<InputStream<'name>>>>,
    outputs: Vec<Option<OutputStream<'name, 'writer>>>,
}

impl<'name, 'writer> PortTable<'name, 'writer> {
    fn with_capacity(streams: usize) -> Self {
        Self {
            inputs: Vec::with_capacity(streams),
            outputs: Vec::with_capacity(streams),
        }
    }

    fn push_stream(
        &mut self,
        readers: Vec<Option<InputStream<'name>>>,
        output: OutputStream<'name, 'writer>,
    ) {
        self.inputs.push(readers);
        self.outputs.push(Some(output));
    }

    fn take_input(&mut self, stream: StreamId) -> Result<InputStream<'name>> {
        self.inputs
            .get_mut(stream.0)
            .and_then(Vec::pop)
            .flatten()
            .ok_or_else(|| {
                WizardError::BuildError(format!("endpoint for stream {stream:?} unavailable"))
            })
    }

    fn take_output(&mut self, stream: StreamId) -> Result<OutputStream<'name, 'writer>> {
        self.outputs
            .get_mut(stream.0)
            .and_then(Option::take)
            .ok_or_else(|| {
                WizardError::BuildError(format!("endpoint for stream {stream:?} unavailable"))
            })
    }

    fn assert_empty(self) -> Result<()> {
        if self
            .inputs
            .iter()
            .any(|slots| slots.iter().any(Option::is_some))
        {
            return Err(WizardError::BuildError(
                "unconsumed stream input endpoint".to_owned(),
            ));
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
    use std::io::{Read as _, Write as _};
    use std::os::unix::net::UnixStream;

    use super::*;
    use crate::artifact::Artifact;
    use crate::pipeline::context::TargetWriters;
    use crate::pipeline::node::PortId;

    fn piped_graph() -> Graph {
        let mut graph = Graph::new();
        let producer = graph.add_node(NodeKind::Concat);
        let consumer = graph.add_node(NodeKind::Uki);
        let stream = graph.add_output(producer, PortId(0)).expect("add output");
        graph.bind_input(consumer, PortId(0), stream).expect("bind");

        graph
    }

    fn fused_graph() -> Graph {
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
        assert_eq!(
            table
                .inputs
                .get(stream.id.0)
                .expect("slot")
                .iter()
                .filter(|slot| slot.is_some())
                .count(),
            1,
            "one reader slot per consumer"
        );
        assert!(table.outputs.get(stream.id.0).expect("slot").is_some());
    }

    #[test]
    fn multi_destination_stream_gets_a_fanout_output_and_one_reader_per_consumer() {
        // ARRANGE
        let mut graph = Graph::new();
        let producer = graph.add_node(NodeKind::KernelPull);
        let first = graph.add_node(NodeKind::Concat);
        let second = graph.add_node(NodeKind::Uki);
        let stream = graph.add_output(producer, PortId(0)).expect("add output");
        graph.bind_input(first, PortId(0), stream).expect("bind");
        graph.bind_input(second, PortId(0), stream).expect("bind");
        let mut writers = TargetWriters::new(Vec::new());

        // ACT
        let mut table = allocate(&graph, &mut writers).expect("allocate");

        // ASSERT
        let output = table.take_output(stream).expect("fanout output");
        assert!(matches!(output.writer, OutputWriter::Fanout(_)));
        table.take_input(stream).expect("first reader");
        table.take_input(stream).expect("second reader");
        assert!(
            table.take_input(stream).is_err(),
            "only one reader per consumer"
        );
    }

    #[test]
    fn fanout_writer_replicates_bytes_to_every_sink() {
        // ARRANGE
        let mut graph = Graph::new();
        let producer = graph.add_node(NodeKind::KernelPull);
        let first = graph.add_node(NodeKind::Concat);
        let second = graph.add_node(NodeKind::Uki);
        let stream = graph.add_output(producer, PortId(0)).expect("add output");
        graph.bind_input(first, PortId(0), stream).expect("bind");
        graph.bind_input(second, PortId(0), stream).expect("bind");
        let mut writers = TargetWriters::new(Vec::new());
        let mut table = allocate(&graph, &mut writers).expect("allocate");
        let mut first_reader = table.take_input(stream).expect("first reader");
        let mut second_reader = table.take_input(stream).expect("second reader");
        let mut output = table.take_output(stream).expect("fanout output");

        // ACT
        output
            .writer
            .write_all(b"replicated")
            .expect("fanout write");
        drop(output);

        // ASSERT
        let mut first = Vec::new();
        first_reader
            .reader
            .read_to_end(&mut first)
            .expect("read first");
        let mut second = Vec::new();
        second_reader
            .reader
            .read_to_end(&mut second)
            .expect("read second");
        assert_eq!(first, b"replicated");
        assert_eq!(second, b"replicated");
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
            table.inputs.get(stream.id.0).expect("slot").is_empty(),
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
            OutputWriter::Pipe(_) | OutputWriter::Fanout(_) => {
                panic!("terminal stream must hold the target writer")
            }
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
