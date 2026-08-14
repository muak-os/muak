//! Attaches the final size and name to every stream before any pipe exists.

use crate::error::Result;
use crate::nodes;
use crate::pipeline::context::BuildContext;
use crate::pipeline::graph::{Graph, NodeKind};

/// Attaches the final size and name to every stream in the normalized graph.
///
/// # Errors
///
/// Returns an error when a source metadata query or size computation fails,
/// or when any stream ended up unnamed.
pub(crate) async fn preflight(
    mut graph: Graph,
    context: &BuildContext<'_, '_, '_>,
) -> Result<(Graph, Vec<mumi::payload::Planned>)> {
    let mut planned_payloads: Vec<mumi::payload::Planned> = Vec::new();

    for id in graph.topological_order() {
        match graph.node(id)?.kind {
            NodeKind::InstallerPull => nodes::installer::preflight(&mut graph, id, context).await?,
            NodeKind::ExtensionPayloads => {
                planned_payloads = nodes::extensions::preflight(&mut graph, id, context).await?;
            }
            NodeKind::InitramfsTail => nodes::initramfs::tail::preflight(&mut graph, id, context)?,
            NodeKind::Concat => nodes::initramfs::concat::preflight(&mut graph, id)?,
            NodeKind::Uki => nodes::uki::preflight(&mut graph, id, context).await?,
            NodeKind::Sign => nodes::sign::preflight(&mut graph, id, context)?,
            NodeKind::OverlayPull => {
                nodes::overlay::pull::preflight(&mut graph, id, context).await?;
            }
            NodeKind::OverlayTar => nodes::overlay::tar::preflight(&mut graph, id)?,
            NodeKind::Fanout => nodes::fanout::preflight(&mut graph, id)?,
            NodeKind::Iso | NodeKind::Raw => nodes::media::preflight(&mut graph, id)?,
            NodeKind::ArtifactSink { .. } => {}
        }
    }

    graph.assert_named()?;

    Ok((graph, planned_payloads))
}
