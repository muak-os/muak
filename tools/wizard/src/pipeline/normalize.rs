//! Rewrites multi-destination streams through explicit fanout nodes.

use crate::artifact::Artifact;
use crate::error::Result;
use crate::nodes::NodeKind;
use crate::nodes::fanout::{FANOUT_INPUT, FANOUT_OUTPUTS_FIRST};
use crate::pipeline::graph::Graph;
use crate::pipeline::node::{NodeId, PortId, StreamId};

/// Ensures every stream has exactly one destination.
///
/// # Errors
///
/// Returns an error when a consumer binding is missing.
pub(crate) fn normalize(graph: &mut Graph) -> Result<()> {
    let stream_count = graph.streams().len();
    for stream_index in 0..stream_count {
        let stream_id = StreamId(stream_index);
        if destinations(graph, stream_id)? <= 1 {
            continue;
        }
        let artifact = graph.stream(stream_id)?.artifact;
        let producer = graph.stream(stream_id)?.producer;
        let fanout = graph.add_node(NodeKind::Fanout);
        graph.bind_input(fanout, FANOUT_INPUT, stream_id)?;
        let fanout = graph.reposition_after(fanout, producer)?;
        let mut output_index = 0;
        for index in (0..graph.nodes().len()).filter(|index| *index != fanout.0) {
            rebind_consumers(graph, NodeId(index), stream_id, fanout, &mut output_index)?;
        }
        stamp_terminal_branch(graph, stream_id, fanout, output_index, artifact)?;
    }

    Ok(())
}

fn destinations(graph: &Graph, stream_id: StreamId) -> Result<usize> {
    let stream = graph.stream(stream_id)?;

    Ok(stream
        .consumers
        .len()
        .saturating_add(usize::from(stream.artifact.is_some())))
}

fn stamp_terminal_branch(
    graph: &mut Graph,
    stream_id: StreamId,
    fanout: NodeId,
    output_index: usize,
    artifact: Option<Artifact>,
) -> Result<()> {
    let Some(artifact) = artifact else {
        return Ok(());
    };
    let port = PortId(FANOUT_OUTPUTS_FIRST.0.saturating_add(output_index));
    let terminal = graph.add_output(fanout, port)?;
    graph.stream_mut(terminal)?.artifact = Some(artifact);
    graph.stream_mut(stream_id)?.artifact = None;

    Ok(())
}

fn rebind_consumers(
    graph: &mut Graph,
    consumer: NodeId,
    stream_id: StreamId,
    fanout: NodeId,
    output_index: &mut usize,
) -> Result<()> {
    while graph
        .node(consumer)?
        .inputs
        .iter()
        .any(|binding| binding.stream == stream_id)
    {
        let port = PortId(FANOUT_OUTPUTS_FIRST.0.saturating_add(*output_index));
        *output_index = output_index.saturating_add(1);
        let output = graph.add_output(fanout, port)?;
        graph.rebind_input(consumer, stream_id, output)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::artifact::Artifact;
    use crate::nodes::NodeKind;
    use crate::pipeline::graph::Graph;
    use crate::pipeline::node::PortId;
    use crate::pipeline::normalize::normalize;

    fn multi_destination_graph() -> Graph {
        // ARRANGE
        let mut graph = Graph::new();
        let producer = graph.add_node(NodeKind::Concat);
        let first = graph.add_node(NodeKind::Uki);
        let stream = graph.add_output(producer, PortId(0)).expect("add output");
        graph.stream_mut(stream).expect("stream").artifact = Some(Artifact::Initramfs);
        graph.bind_input(first, PortId(0), stream).expect("bind");

        graph
    }

    #[test]
    fn inserts_fanout_for_multi_destination_stream() {
        // ARRANGE
        let mut graph = multi_destination_graph();
        let original = graph.streams().iter().next().expect("stream").id;

        // ACT
        normalize(&mut graph).expect("normalize");

        // ASSERT
        let stream = graph.stream(original).expect("stream");
        assert_eq!(stream.consumers.len(), 1);
        assert_eq!(stream.artifact, None, "stamp must move to the branch");
        let fanout = stream.consumers.first().expect("consumer");
        let node = graph.node(*fanout).expect("node");
        assert!(matches!(&node.kind, NodeKind::Fanout));
        assert_eq!(node.inputs.len(), 1);
        assert_eq!(node.outputs.len(), 2);
        let branches: Vec<_> = node
            .outputs
            .iter()
            .map(|binding| graph.stream(binding.stream).expect("stream"))
            .collect();
        assert_eq!(
            branches
                .iter()
                .filter(|branch| branch.consumers.len() == 1)
                .count(),
            1,
            "exactly one piped branch"
        );
        assert_eq!(
            branches
                .iter()
                .filter(|branch| branch.consumers.is_empty())
                .count(),
            1,
            "exactly one terminal branch"
        );
        let terminal = branches
            .iter()
            .find(|branch| branch.consumers.is_empty())
            .expect("terminal branch");
        assert_eq!(
            terminal.artifact,
            Some(Artifact::Initramfs),
            "the terminal branch must carry the stamp"
        );
    }

    #[test]
    fn leaves_single_destination_streams_alone() {
        // ARRANGE
        let mut graph = Graph::new();
        let producer = graph.add_node(NodeKind::Concat);
        let consumer = graph.add_node(NodeKind::Uki);
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

    #[test]
    fn normalized_node_ids_stay_producer_first() {
        // ARRANGE
        let mut graph = multi_destination_graph();

        // ACT
        normalize(&mut graph).expect("normalize");

        // ASSERT
        for stream in graph.streams() {
            assert!(
                stream
                    .consumers
                    .iter()
                    .all(|consumer| stream.producer.0 < consumer.0),
                "producer {:?} must precede all its consumers",
                stream.producer
            );
        }
    }

    #[test]
    fn keeps_a_pure_terminal_stream_untouched() {
        // ARRANGE
        let mut graph = Graph::new();
        let producer = graph.add_node(NodeKind::KernelPull);
        let stream = graph.add_output(producer, PortId(0)).expect("add output");
        graph.stream_mut(stream).expect("stream").artifact = Some(Artifact::Kernel);

        // ACT
        normalize(&mut graph).expect("normalize");

        // ASSERT
        assert_eq!(graph.nodes().len(), 1, "no fanout for a lone terminal");
        let stamped = graph
            .streams()
            .iter()
            .find(|stream| stream.artifact == Some(Artifact::Kernel))
            .expect("stamped stream");
        assert!(
            graph
                .stream(stamped.id)
                .expect("stream")
                .consumers
                .is_empty(),
            "a lone terminal must keep no consumer"
        );
    }

    #[test]
    fn terminal_branch_copies_the_stamp_of_a_multi_consumer_stream() {
        // ARRANGE
        let mut graph = Graph::new();
        let producer = graph.add_node(NodeKind::KernelPull);
        let first = graph.add_node(NodeKind::Concat);
        let stream = graph.add_output(producer, PortId(0)).expect("add output");
        graph.stream_mut(stream).expect("stream").artifact = Some(Artifact::Cmdline);
        graph.bind_input(first, PortId(0), stream).expect("bind");

        // ACT
        normalize(&mut graph).expect("normalize");

        // ASSERT
        let stamped = graph
            .streams()
            .iter()
            .filter(|stream| stream.artifact == Some(Artifact::Cmdline))
            .count();
        assert_eq!(
            stamped, 1,
            "exactly one terminal branch must carry the stamp"
        );
        let fanout = graph
            .nodes()
            .iter()
            .find(|node| node.kind == NodeKind::Fanout)
            .expect("fanout node");
        let terminal = fanout
            .outputs
            .iter()
            .map(|binding| graph.stream(binding.stream).expect("stream"))
            .find(|stream| stream.artifact == Some(Artifact::Cmdline))
            .expect("terminal branch");
        assert!(terminal.consumers.is_empty());
    }
}
