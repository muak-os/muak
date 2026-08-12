//! Rewrites multi-consumer streams through explicit fanout nodes.

use crate::error::Result;
use crate::nodes::fanout::{FANOUT_INPUT, FANOUT_OUTPUTS_FIRST};
use crate::pipeline::graph::{Graph, NodeKind, PortId};

/// Ensures every stream has exactly one consumer.
///
/// # Errors
///
/// Returns an error when a consumer binding is missing.
pub(crate) fn normalize(graph: &mut Graph) -> Result<()> {
    let stream_ids = graph
        .streams()
        .iter()
        .map(|stream| stream.id)
        .collect::<Vec<_>>();
    for stream_id in stream_ids {
        if graph.stream(stream_id)?.consumers.len() <= 1 {
            continue;
        }
        let consumers = graph.stream(stream_id)?.consumers.clone();
        let fanout = graph.add_node(NodeKind::Fanout);
        graph.bind_input(fanout, FANOUT_INPUT, stream_id)?;
        for (index, consumer) in consumers.iter().enumerate() {
            let output =
                graph.add_output(fanout, PortId(FANOUT_OUTPUTS_FIRST.0.saturating_add(index)))?;
            graph.rebind_input(*consumer, stream_id, output)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::artifact::Artifact;
    use crate::pipeline::graph::{Graph, NodeKind, PortId};
    use crate::pipeline::normalize::normalize;

    fn multi_consumer_graph() -> Graph {
        // ARRANGE
        let mut graph = Graph::new();
        let producer = graph.add_node(NodeKind::Concat);
        let first = graph.add_node(NodeKind::ArtifactSink {
            artifact: Artifact::Initramfs,
        });
        let second = graph.add_node(NodeKind::Uki);
        let stream = graph.add_output(producer, PortId(0)).expect("add output");
        graph.bind_input(first, PortId(0), stream).expect("bind");
        graph.bind_input(second, PortId(0), stream).expect("bind");

        graph
    }

    #[test]
    fn inserts_fanout_for_multi_consumer_stream() {
        // ARRANGE
        let mut graph = multi_consumer_graph();
        let original = graph.streams().iter().next().expect("stream").id;

        // ACT
        normalize(&mut graph).expect("normalize");

        // ASSERT
        let stream = graph.stream(original).expect("stream");
        assert_eq!(stream.consumers.len(), 1);
        let fanout = stream.consumers.first().expect("consumer");
        let node = graph.node(*fanout).expect("node");
        assert!(matches!(&node.kind, NodeKind::Fanout));
        assert_eq!(node.inputs.len(), 1);
        assert_eq!(node.outputs.len(), 2);
        for binding in &node.outputs {
            assert_eq!(
                graph
                    .stream(binding.stream)
                    .expect("stream")
                    .consumers
                    .len(),
                1
            );
        }
    }

    #[test]
    fn leaves_single_consumer_streams_alone() {
        // ARRANGE
        let mut graph = Graph::new();
        let producer = graph.add_node(NodeKind::Concat);
        let consumer = graph.add_node(NodeKind::ArtifactSink {
            artifact: Artifact::Initramfs,
        });
        let stream = graph.add_output(producer, PortId(0)).expect("add output");
        graph.bind_input(consumer, PortId(0), stream).expect("bind");

        // ACT
        normalize(&mut graph).expect("normalize");

        // ASSERT
        assert_eq!(graph.nodes().len(), 2);
        assert_eq!(
            graph.stream(stream).expect("stream").consumers,
            vec![consumer]
        );
    }
}
