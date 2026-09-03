//! Generic scoped-thread executor for normalized preflighted graphs.

use std::thread;

use crate::error::Result;
use crate::nodes;
use crate::pipeline::context::{BuildContext, TargetWriters};
use crate::pipeline::graph::Graph;
use crate::pipeline::preflight;
use crate::pipeline::prepare::{PreparedNode, bind_nodes};
use crate::{Metadata, SectionInfo};

/// What a node reports on success: PE sections when it produced any.
pub(crate) type NodeReport = Option<Vec<SectionInfo>>;

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
            joins.push(scope.spawn(move || node.run(ctx)));
        }
        join_all(joins)
    })
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
