//! Validates normalized-graph invariants.

use crate::error::{Result, WizardError};
use crate::pipeline::graph::{Graph, PortBinding, PortId, Stream};

/// Checks the normalized-graph invariants the executor relies on.
///
/// # Errors
///
/// Returns an error on the first violation.
pub(crate) fn normalized(graph: &Graph) -> Result<()> {
    for stream in graph.streams() {
        let consumer_count = stream.consumers.len();
        if consumer_count == 0 {
            return Err(WizardError::BuildError(format!(
                "stream {:?} has no consumer",
                stream.id
            )));
        }
        if consumer_count > 1 {
            return Err(WizardError::BuildError(format!(
                "stream {:?} has {consumer_count} consumers",
                stream.id
            )));
        }
        let producer = graph.node(stream.producer)?;
        if !producer
            .outputs
            .iter()
            .any(|binding| binding.stream == stream.id)
        {
            return Err(WizardError::BuildError(format!(
                "stream {:?} is not listed among producer outputs",
                stream.id
            )));
        }
        consumers_consistent(graph, stream)?;
    }

    for node in graph.nodes() {
        if let Some(port) = duplicate_port(&node.inputs) {
            return Err(WizardError::BuildError(format!(
                "node {:?} binds input port {port:?} twice",
                node.id
            )));
        }
        if let Some(port) = duplicate_port(&node.outputs) {
            return Err(WizardError::BuildError(format!(
                "node {:?} binds output port {port:?} twice",
                node.id
            )));
        }
        for binding in node.inputs.iter().chain(&node.outputs) {
            graph.stream(binding.stream)?;
        }
    }

    Ok(())
}

fn consumers_consistent(graph: &Graph, stream: &Stream) -> Result<()> {
    for consumer in &stream.consumers {
        let node = graph.node(*consumer)?;
        if !node
            .inputs
            .iter()
            .any(|binding| binding.stream == stream.id)
        {
            return Err(WizardError::BuildError(format!(
                "stream {:?} is not listed among consumer inputs",
                stream.id
            )));
        }
    }

    Ok(())
}

fn duplicate_port(bindings: &[PortBinding]) -> Option<PortId> {
    bindings.iter().enumerate().find_map(|(index, left)| {
        bindings
            .iter()
            .skip(index.saturating_add(1))
            .find(|right| right.port == left.port)
            .map(|_| left.port)
    })
}

#[cfg(test)]
mod tests {
    use crate::artifact::Artifact;
    use crate::pipeline::graph::{Graph, NodeKind, PortBinding, PortId, StreamId};
    use crate::pipeline::validate::normalized;

    fn valid_graph() -> Graph {
        // ARRANGE
        let mut graph = Graph::new();
        let producer = graph.add_node(NodeKind::InstallerPull);
        let consumer = graph.add_node(NodeKind::ArtifactSink {
            artifact: Artifact::Kernel,
        });
        let stream = graph.add_output(producer, PortId(0)).expect("add output");
        graph.bind_input(consumer, PortId(0), stream).expect("bind");

        graph
    }

    #[test]
    fn accepts_normalized_graph() {
        // ARRANGE
        let graph = valid_graph();

        // ACT
        let result = normalized(&graph);

        // ASSERT
        result.unwrap();
    }

    #[test]
    fn rejects_stream_without_consumer() {
        // ARRANGE
        let mut graph = valid_graph();
        graph
            .add_output(graph.nodes().iter().next().expect("node").id, PortId(1))
            .expect("add output");

        // ACT
        let result = normalized(&graph);

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn rejects_stream_with_multiple_consumers() {
        // ARRANGE
        let mut graph = valid_graph();
        let producer = graph.nodes().iter().next().expect("node").id;
        let extra = graph.add_node(NodeKind::Uki);
        let stream = graph.add_output(producer, PortId(1)).expect("add output");
        graph.bind_input(extra, PortId(0), stream).expect("bind");
        graph.bind_input(extra, PortId(1), stream).expect("bind");

        // ACT
        let result = normalized(&graph);

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn rejects_duplicate_port() {
        // ARRANGE
        let mut graph = valid_graph();
        let consumer = graph.nodes().iter().last().expect("node").id;
        let stream = graph.streams().iter().next().expect("stream").id;
        graph
            .node_mut(consumer)
            .expect("node")
            .inputs
            .push(PortBinding {
                port: PortId(0),
                stream,
            });

        // ACT
        let result = normalized(&graph);

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn rejects_binding_to_missing_stream() {
        // ARRANGE
        let mut graph = valid_graph();
        let consumer = graph.nodes().iter().last().expect("node").id;
        graph
            .node_mut(consumer)
            .expect("node")
            .inputs
            .push(PortBinding {
                port: PortId(1),
                stream: StreamId(99),
            });

        // ACT
        let result = normalized(&graph);

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn rejects_producer_stream_list_mismatch() {
        // ARRANGE
        let mut graph = valid_graph();
        let producer = graph.nodes().iter().next().expect("node").id;
        let stream = graph.streams().iter().next().expect("stream").id;
        graph
            .node_mut(producer)
            .expect("node")
            .outputs
            .retain(|binding| binding.stream != stream);

        // ACT
        let result = normalized(&graph);

        // ASSERT
        result.unwrap_err();
    }
}
