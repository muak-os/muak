//! Attaches the final size to every stream and collects the dynamic per-stream data lists before any pipe exists.

use crate::error::Result;
use crate::nodes;
use crate::pipeline::context::BuildContext;
use crate::pipeline::graph::{Graph, NodeKind};

/// The graph plus all dynamic data discovered before runtime binding.
pub(crate) struct PreflightedGraph {
    pub(crate) graph: Graph,
    /// Pre-planned extension payloads in canonical source order.
    pub(crate) planned_payloads: Vec<mumi::payload::Planned>,
    /// Stripped overlay file paths plus sizes, path-sorted.
    pub(crate) overlay_files: Vec<(String, u64)>,
}

/// Attaches the final size to every stream in the normalized graph.
///
/// # Errors
///
/// Returns an error when a source metadata query or size computation fails.
pub(crate) async fn preflight(
    mut graph: Graph,
    context: &BuildContext<'_, '_>,
) -> Result<PreflightedGraph> {
    let mut planned_payloads: Vec<mumi::payload::Planned> = Vec::new();
    let mut overlay_files: Vec<(String, u64)> = Vec::new();

    for id in graph.topological_order() {
        match graph.node(id)?.kind {
            NodeKind::InstallerPull => nodes::installer::preflight(&mut graph, id, context).await?,
            NodeKind::ExtensionPayloads => {
                planned_payloads = nodes::extensions::preflight(&mut graph, id, context).await?;
            }
            NodeKind::InitramfsTail => {
                nodes::initramfs::preflight_tail(&mut graph, id, context, &planned_payloads)?;
            }
            NodeKind::Concat => nodes::initramfs::preflight_concat(&mut graph, id)?,
            NodeKind::Uki => nodes::uki::preflight(&mut graph, id, context).await?,
            NodeKind::OverlayPull => {
                overlay_files = nodes::overlays::preflight_pull(&mut graph, id, context).await?;
            }
            NodeKind::OverlayTar => nodes::overlays::preflight_tar(&mut graph, id)?,
            NodeKind::Fanout => nodes::fanout::preflight(&mut graph, id)?,
            NodeKind::Iso | NodeKind::Raw | NodeKind::ArtifactSink { .. } => {}
        }
    }

    Ok(PreflightedGraph {
        graph,
        planned_payloads,
        overlay_files,
    })
}
