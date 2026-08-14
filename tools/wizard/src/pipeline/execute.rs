//! Generic scoped-thread executor for normalized preflighted graphs.

use std::thread;

use tokio::task::block_in_place;

use crate::error::Result;
use crate::nodes;
use crate::pipeline::context::BuildContext;
use crate::pipeline::graph::{Graph, NodeKind};
use crate::pipeline::preflight::{self, PreflightedGraph};
use crate::pipeline::prepare::bind_nodes;
use crate::pipeline::runtime::PreparedNode;
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
pub(crate) async fn execute(graph: Graph, context: &BuildContext<'_, '_, '_>) -> Result<Metadata> {
    validate::normalized(&graph)?;
    let preflighted = preflight::preflight(graph, context).await?;
    block_in_place(|| execute_blocking(preflighted, context))
}

fn execute_blocking(
    preflighted: PreflightedGraph,
    context: &BuildContext<'_, '_, '_>,
) -> Result<Metadata> {
    let tokio = tokio::runtime::Handle::current();
    let (nodes, planned_payloads, overlay_files) = bind_nodes(preflighted)?;

    thread::scope(|scope| {
        let planned = &planned_payloads;
        let overlays = &overlay_files;
        let mut joins = Vec::with_capacity(nodes.len());
        for node in nodes {
            let tokio = tokio.clone();
            joins.push(scope.spawn(move || node.run(context, planned, overlays, &tokio)));
        }
        join_all(joins)
    })
}

impl PreparedNode {
    /// Dispatches the node logic by kind and runs it on its own thread.
    fn run(
        self,
        ctx: &BuildContext<'_, '_, '_>,
        planned: &[mumi::payload::Planned],
        overlay_files: &[(String, u64)],
        tokio: &tokio::runtime::Handle,
    ) -> Result<NodeReport> {
        let PreparedNode { kind, mut ports } = self;
        match kind {
            NodeKind::InstallerPull => nodes::installer::run(ctx, &mut ports, tokio),
            NodeKind::ExtensionPayloads => nodes::extensions::run(planned, &mut ports),
            NodeKind::InitramfsTail => nodes::initramfs::tail::run(ctx, planned, &mut ports),
            NodeKind::Concat => nodes::initramfs::concat::run(&mut ports),
            NodeKind::Uki => nodes::uki::run(ctx, &mut ports),
            NodeKind::Sign => nodes::sign::run(ctx, &mut ports),
            NodeKind::Iso => nodes::media::run_iso(ctx, overlay_files, &mut ports),
            NodeKind::Raw => nodes::media::run_raw(ctx, overlay_files, &mut ports),
            NodeKind::OverlayPull => {
                nodes::overlay::pull::run(ctx, overlay_files, &mut ports, tokio)
            }
            NodeKind::OverlayTar => nodes::overlay::tar::run(overlay_files, &mut ports),
            NodeKind::ArtifactSink { artifact } => nodes::sink::run(ctx, artifact, &mut ports),
            NodeKind::Fanout => nodes::fanout::run(&mut ports),
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
