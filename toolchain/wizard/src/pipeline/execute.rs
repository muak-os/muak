//! Generic scoped-thread executor for normalized preflighted graphs.

use std::thread;

use uki::measure::MeasuredSection;

use crate::Metadata;
use crate::artifact::Artifact;
use crate::codec::Codec;
use crate::error::Result;
use crate::nodes;
use crate::pipeline::context::{BuildContext, TargetWriters};
use crate::pipeline::graph::Graph;
use crate::pipeline::preflight;
use crate::pipeline::prepare::{PreparedNode, bind_nodes};
use crate::progress::{Event, NodeState};

/// What a node reports on success: PE sections when it produced any.
pub(crate) type NodeReport = Option<Vec<MeasuredSection>>;

/// Preflights and executes the normalized graph.
///
/// # Errors
///
/// Returns the first meaningful node error after joining every thread.
pub(crate) fn execute(
    graph: Graph,
    ctx: &BuildContext<'_, '_>,
    writers: &mut TargetWriters<'_>,
) -> Result<Metadata> {
    let graph = preflight::preflight(graph, ctx)?;
    let (artifacts, bytes) = artifact_totals(&graph, ctx.codec);
    ctx.progress.report(Event::Sized { artifacts, bytes });

    execute_blocking(&graph, ctx, writers)
}

fn execute_blocking(
    graph: &Graph,
    ctx: &BuildContext<'_, '_>,
    writers: &mut TargetWriters<'_>,
) -> Result<Metadata> {
    let nodes = bind_nodes(graph, writers)?;

    thread::scope(|scope| {
        let mut joins = Vec::with_capacity(nodes.len());
        for node in nodes {
            let kind = node.kind;
            joins.push(scope.spawn(move || run_node(node, kind, ctx)));
        }

        join_all(joins)
    })
}

fn run_node(
    node: PreparedNode<'_, '_>,
    kind: nodes::NodeKind,
    ctx: &BuildContext<'_, '_>,
) -> Result<NodeReport> {
    ctx.progress.report(Event::Node {
        node: kind.title(),
        state: NodeState::Started,
    });
    let report = node.run(ctx);
    let state = if report.is_ok() {
        NodeState::Completed
    } else {
        NodeState::Failed
    };
    ctx.progress.report(Event::Node {
        node: kind.title(),
        state,
    });

    report
}

impl PreparedNode<'_, '_> {
    fn run(self, ctx: &BuildContext<'_, '_>) -> Result<NodeReport> {
        let PreparedNode { kind, mut ports } = self;
        let node = nodes::descriptor(kind);

        (node.run)(kind, &mut ports, ctx)
    }
}

fn join_all(joins: Vec<thread::ScopedJoinHandle<'_, Result<NodeReport>>>) -> Result<Metadata> {
    let mut report = Metadata::default();
    let mut first_error = None;

    for join in joins {
        match join.join() {
            Ok(Ok(Some(sections))) => report.sections.extend(sections),
            Ok(Ok(None)) => {}
            Ok(Err(error)) => {
                first_error.get_or_insert(error);
            }
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }

    if let Some(error) = first_error {
        return Err(error);
    }

    Ok(report)
}

fn artifact_totals(graph: &Graph, codec: Codec) -> (usize, Option<u64>) {
    let mut count: usize = 0;
    let mut bytes: u64 = 0;
    let mut known = true;
    for stream in graph
        .streams()
        .iter()
        .filter(|stream| stream.artifact.is_some())
    {
        count = count.saturating_add(1);
        bytes = bytes.saturating_add(stream.size);
        if stream.size == 0 || (stream.artifact == Some(Artifact::Raw) && codec.compresses()) {
            known = false;
        }
    }

    (count, known.then_some(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::Artifact;
    use crate::nodes::NodeKind;
    use crate::pipeline::node::PortId;

    fn graph(size: u64, artifact: Artifact) -> Graph {
        let mut graph = Graph::new();
        let node = graph.add_node(NodeKind::Uki);
        let terminal = graph.add_output(node, PortId(0)).expect("terminal stream");
        let internal = graph.add_output(node, PortId(1)).expect("internal stream");
        graph.stream_mut(terminal).expect("stream").size = size;
        graph.stream_mut(internal).expect("stream").size = 5;
        graph.stream_mut(terminal).expect("stream").artifact = Some(artifact);

        graph
    }

    #[test]
    fn artifact_totals_sum_only_terminal_streams() {
        // ARRANGE
        let graph = graph(10, Artifact::Uki);

        // ACT
        let (count, bytes) = artifact_totals(&graph, Codec::None);

        // ASSERT
        assert_eq!(count, 1);
        assert_eq!(bytes, Some(10));
    }

    #[test]
    fn zero_sized_artifacts_are_not_sized() {
        // ARRANGE
        let graph = graph(0, Artifact::Uki);

        // ACT
        let (_, bytes) = artifact_totals(&graph, Codec::None);

        // ASSERT
        assert_eq!(bytes, None, "unknown media sizes must not drive a bar");
    }

    #[test]
    fn compressed_raw_is_not_sized() {
        // ARRANGE
        let graph = graph(10, Artifact::Raw);

        // ACT
        let compressed = artifact_totals(&graph, Codec::Zstd);
        let plain = artifact_totals(&graph, Codec::None);

        // ASSERT
        assert_eq!(
            compressed.1, None,
            "compressed output size is not the plan size"
        );
        assert_eq!(plain.1, Some(10));
    }
}
