//! Attaches the final size and name to every stream before any pipe exists.

use crate::error::Result;
use crate::nodes;
use crate::pipeline::context::BuildContext;
use crate::pipeline::graph::Graph;

/// Attaches the final size and name to every stream in the normalized graph.
///
/// # Errors
///
/// Returns an error when a source metadata query or size computation fails,
/// or when any stream ended up unnamed.
pub(crate) fn preflight(graph: Graph, ctx: &BuildContext<'_, '_>) -> Result<Graph> {
    let mut graph = graph;

    for id in graph.topological_order() {
        let kind = graph.node(id)?.kind;
        let node = nodes::descriptor(kind);
        (node.preflight)(&mut graph, id, ctx)?;
    }

    graph.assert_named()?;

    Ok(graph)
}
