//! Kahn's algorithm: deterministic producer-before-consumer order.

use alloc::collections::VecDeque;

use crate::pipeline::graph::{Node, NodeId, Stream};

/// Produces every node id in producer-before-consumer order.
#[must_use]
pub(crate) fn topological(nodes: &[Node], streams: &[Stream]) -> Vec<NodeId> {
    let mut indegree = indegrees(nodes, streams);
    let mut ready = ready_queue(&indegree);
    let mut order = Vec::with_capacity(nodes.len());
    drain(nodes, streams, &mut indegree, &mut ready, &mut order);

    order
}

fn indegrees(nodes: &[Node], streams: &[Stream]) -> Vec<usize> {
    let mut indegree: Vec<usize> = vec![0; nodes.len()];
    for consumer in streams.iter().flat_map(|stream| &stream.consumers) {
        if let Some(degree) = indegree.get_mut(consumer.0) {
            *degree = degree.saturating_add(1);
        }
    }

    indegree
}

fn ready_queue(indegree: &[usize]) -> VecDeque<usize> {
    indegree
        .iter()
        .enumerate()
        .filter(|item| *item.1 == 0)
        .map(|(index, _)| index)
        .collect()
}

fn drain(
    nodes: &[Node],
    streams: &[Stream],
    indegree: &mut [usize],
    ready: &mut VecDeque<usize>,
    order: &mut Vec<NodeId>,
) {
    while let Some(index) = ready.pop_front() {
        order.push(NodeId(index));
        for consumer in node_consumers(nodes, streams, index) {
            advance(indegree, ready, consumer);
        }
    }
}

fn node_consumers(nodes: &[Node], streams: &[Stream], index: usize) -> Vec<NodeId> {
    nodes
        .get(index)
        .map(|node| {
            node.outputs
                .iter()
                .filter_map(|binding| streams.get(binding.stream.0))
                .flat_map(|stream| &stream.consumers)
                .copied()
                .collect()
        })
        .unwrap_or_default()
}

fn advance(indegree: &mut [usize], ready: &mut VecDeque<usize>, consumer: NodeId) {
    if let Some(degree) = indegree.get_mut(consumer.0) {
        *degree = degree.saturating_sub(1);
        if *degree == 0 {
            ready.push_back(consumer.0);
        }
    }
}
