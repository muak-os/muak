//! Plan-time gate for terminal artifact streams.

use crate::error::{Result, WizardError};
use crate::pipeline::context::TargetWriters;
use crate::pipeline::graph::Graph;

/// Rejects terminal streams that also feed nodes and stamps without writers.
///
/// # Errors
///
/// Returns an error when a stamped stream feeds a node, or when no writer
/// exists for a stamped artifact.
pub(crate) fn terminate(graph: &Graph, writers: &TargetWriters<'_>) -> Result<()> {
    for stream in graph.streams() {
        let Some(artifact) = stream.artifact else {
            continue;
        };
        if !stream.consumers.is_empty() {
            return Err(WizardError::BuildError(format!(
                "terminal stream for {artifact} also feeds {} node(s)",
                stream.consumers.len()
            )));
        }
        if !writers.available(artifact) {
            return Err(WizardError::BuildError(format!(
                "missing target writer for {artifact}"
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::artifact::Artifact;
    use crate::nodes::NodeKind;
    use crate::pipeline::graph::{Graph, PortId};

    struct Sink;

    impl Write for Sink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn kernel_graph(stamped: bool) -> Graph {
        // ARRANGE
        let mut graph = Graph::new();
        let producer = graph.add_node(NodeKind::KernelPull);
        let stream = graph.add_output(producer, PortId(0)).expect("add output");
        if stamped {
            graph.stream_mut(stream).expect("stream").artifact = Some(Artifact::Kernel);
        }

        graph
    }

    fn writers<'a>(artifacts: &[Artifact], sinks: &'a mut [Sink]) -> TargetWriters<'a> {
        // ARRANGE
        let pairs: Vec<(Artifact, &'a mut (dyn Write + Send))> = artifacts
            .iter()
            .zip(sinks.iter_mut())
            .map(|(artifact, sink)| {
                let writer: &'a mut (dyn Write + Send) = sink;
                (*artifact, writer)
            })
            .collect();
        TargetWriters::new(pairs)
    }

    #[test]
    fn accepts_an_unstamped_graph_without_writers() {
        // ARRANGE
        let graph = kernel_graph(false);
        let mut sinks: [Sink; 0] = [];
        let writers = writers(&[], &mut sinks);

        // ACT
        let result = terminate(&graph, &writers);

        // ASSERT
        result.expect("unstamped graph");
    }

    #[test]
    fn accepts_a_stamped_stream_with_a_writer() {
        // ARRANGE
        let graph = kernel_graph(true);
        let mut sinks = [Sink];
        let writers = writers(&[Artifact::Kernel], &mut sinks);

        // ACT
        let result = terminate(&graph, &writers);

        // ASSERT
        result.expect("terminal stream with writer");
    }

    #[test]
    fn rejects_a_stamped_stream_without_a_writer() {
        // ARRANGE
        let graph = kernel_graph(true);
        let mut sinks = [Sink];
        let writers = writers(&[Artifact::Cmdline], &mut sinks);

        // ACT
        let error = terminate(&graph, &writers).expect_err("missing writer");

        // ASSERT
        let message = error.to_string();
        assert!(
            message.contains("missing target writer for kernel"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn rejects_a_stamped_stream_that_also_feeds_a_node() {
        // ARRANGE
        let mut graph = kernel_graph(true);
        let consumer = graph.add_node(NodeKind::Concat);
        let stream = graph.streams().iter().next().expect("stream").id;
        graph
            .bind_input(consumer, PortId(0), stream)
            .expect("bind input");
        let mut sinks = [Sink];
        let writers = writers(&[Artifact::Kernel], &mut sinks);

        // ACT
        let error = terminate(&graph, &writers).expect_err("feeding terminal");

        // ASSERT
        let message = error.to_string();
        assert!(
            message.contains("terminal stream for kernel also feeds"),
            "unexpected error: {message}"
        );
    }
}
