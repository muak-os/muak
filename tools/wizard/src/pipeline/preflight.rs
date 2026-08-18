//! Attaches the final size and name to every stream before any pipe exists.

use crate::error::Result;
use crate::nodes::{self, NodeKind};
use crate::pipeline::context::BuildContext;
use crate::pipeline::graph::Graph;

/// Attaches the final size and name to every stream in the normalized graph.
///
/// # Errors
///
/// Returns an error when a source metadata query or size computation fails,
/// or when any stream ended up unnamed.
pub(crate) fn preflight(
    mut graph: Graph,
    ctx: &BuildContext<'_, '_, '_>,
) -> Result<(Graph, Vec<mumi::payload::Planned>)> {
    let mut planned_payloads: Vec<mumi::payload::Planned> = Vec::new();

    for id in graph.topological_order() {
        let kind = graph.node(id)?.kind;
        if let NodeKind::ExtensionPayloads = kind {
            planned_payloads = nodes::extensions::preflight(&mut graph, id, ctx)?;
        } else {
            let node = nodes::descriptor(kind);
            (node.preflight)(&mut graph, id, ctx)?;
        }
    }

    graph.assert_named()?;

    Ok((graph, planned_payloads))
}
