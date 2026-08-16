//! Generic scoped-thread executor for normalized preflighted graphs.

use std::thread;

use crate::error::Result;
use crate::nodes::{self, NodeKind};
use crate::pipeline::context::BuildContext;
use crate::pipeline::graph::Graph;
use crate::pipeline::preflight;
use crate::pipeline::prepare::{PreparedNode, bind_nodes};
use crate::pipeline::validate;
use crate::{Metadata, SectionInfo};

/// Per-node result collected when the node's thread is joined.
pub(crate) enum NodeReport {
    Empty,
    Uki(Vec<SectionInfo>),
}

/// Validates, preflights, and executes the normalized graph.
///
/// # Errors
///
/// Returns the first meaningful node error after joining every thread.
pub(crate) fn execute(graph: Graph, ctx: &BuildContext<'_, '_, '_>) -> Result<Metadata> {
    validate::normalized(&graph)?;
    let (graph, planned_payloads) = preflight::preflight(graph, ctx)?;
    execute_blocking(&graph, &planned_payloads, ctx)
}

fn execute_blocking(
    graph: &Graph,
    planned_payloads: &[mumi::payload::Planned],
    ctx: &BuildContext<'_, '_, '_>,
) -> Result<Metadata> {
    let nodes = bind_nodes(graph)?;

    thread::scope(|scope| {
        let planned = &planned_payloads;
        let mut joins = Vec::with_capacity(nodes.len());
        for node in nodes {
            joins.push(scope.spawn(move || node.run(ctx, planned)));
        }
        join_all(joins)
    })
}

impl PreparedNode<'_> {
    /// Dispatches the node logic by kind and runs it on its own thread.
    fn run(
        self,
        ctx: &BuildContext<'_, '_, '_>,
        planned: &[mumi::payload::Planned],
    ) -> Result<NodeReport> {
        let PreparedNode { kind, mut ports } = self;
        if let NodeKind::ArtifactSink { artifact } = kind {
            nodes::sink::run(ctx, artifact, &mut ports)
        } else if kind == NodeKind::ExtensionPayloads {
            nodes::extensions::run(planned, &mut ports)
        } else {
            let node = nodes::descriptor(kind)?;
            (node.run)(&mut ports, ctx)
        }
    }
}

/// Joins every thread, collecting metadata and the first error.
///
/// Joining continues after the first error: other nodes may need their pipe
/// ends to close before the executor can safely return.
fn join_all(joins: Vec<thread::ScopedJoinHandle<'_, Result<NodeReport>>>) -> Result<Metadata> {
    let mut report = Metadata::default();
    let mut first_error = None;

    for join in joins {
        match join.join() {
            Ok(Ok(NodeReport::Uki(sections))) => report.sections.extend(sections),
            Ok(Ok(NodeReport::Empty)) => {}
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
