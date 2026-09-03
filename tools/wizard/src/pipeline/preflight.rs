//! Attaches the final size and name to every stream before any pipe exists.

use crate::error::Result;
use crate::nodes;
use crate::pipeline::context::BuildContext;
use crate::pipeline::graph::Graph;
use crate::pipeline::node::NodeId;

/// Attaches the final size and name to every stream in the normalized graph.
///
/// # Errors
///
/// Returns an error when a source metadata query or size computation fails,
/// or when any stream ended up unnamed.
pub(crate) fn preflight(graph: Graph, ctx: &BuildContext<'_, '_>) -> Result<Graph> {
    let mut graph = graph;

    for index in 0..graph.nodes().len() {
        let kind = graph.node(NodeId(index))?.kind;
        let node = nodes::descriptor(kind);
        (node.preflight)(&mut graph, NodeId(index), ctx)?;
    }

    graph.assert_named()?;

    Ok(graph)
}
