//! Logical build graph.

use crate::error::{Result, WizardError};
use crate::nodes::NodeKind;
use crate::pipeline::node::{Node, NodeId, PortBinding, PortId, Stream, StreamId};

/// The logical build DAG: nodes plus the streams between them.
#[derive(Clone, Debug, Default)]
pub(crate) struct Graph {
    nodes: Vec<Node>,
    streams: Vec<Stream>,
}

impl Graph {
    /// Creates an empty graph.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Appends a node and returns its id.
    pub(crate) fn add_node(&mut self, kind: NodeKind) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.nodes.push(Node {
            id,
            kind,
            inputs: Vec::new(),
            outputs: Vec::new(),
        });

        id
    }

    /// Creates a stream produced by `node` and binds it to `port`.
    pub(crate) fn add_output(&mut self, node: NodeId, port: PortId) -> Result<StreamId> {
        let id = StreamId(self.streams.len());
        self.streams.push(Stream {
            id,
            name: String::new(),
            size: 0,
            consumers: Vec::new(),
            artifact: None,
        });
        let node_ref = self.node_mut(node)?;
        if node_ref.outputs.iter().any(|binding| binding.port == port) {
            return Err(WizardError::BuildError(format!(
                "duplicate output port {port:?} on node {node:?}"
            )));
        }
        node_ref.outputs.push(PortBinding { port, stream: id });

        Ok(id)
    }

    /// Binds an input port of `node` to an existing stream.
    pub(crate) fn bind_input(
        &mut self,
        node: NodeId,
        port: PortId,
        stream: StreamId,
    ) -> Result<()> {
        let node_ref = self.node_mut(node)?;
        if node_ref.inputs.iter().any(|binding| binding.port == port) {
            return Err(WizardError::BuildError(format!(
                "duplicate input port {port:?} on node {node:?}"
            )));
        }
        node_ref.inputs.push(PortBinding { port, stream });
        self.stream_mut(stream)?.consumers.push(node);

        Ok(())
    }

    /// Returns the node with the given id.
    pub(crate) fn node(&self, id: NodeId) -> Result<&Node> {
        Self::get(&self.nodes, "node", id.0)
    }

    /// Returns the node with the given id, mutably.
    pub(crate) fn node_mut(&mut self, id: NodeId) -> Result<&mut Node> {
        Self::get_mut(&mut self.nodes, "node", id.0)
    }

    /// Returns the stream with the given id.
    pub(crate) fn stream(&self, id: StreamId) -> Result<&Stream> {
        Self::get(&self.streams, "stream", id.0)
    }

    /// Returns the stream with the given id, mutably.
    pub(crate) fn stream_mut(&mut self, id: StreamId) -> Result<&mut Stream> {
        Self::get_mut(&mut self.streams, "stream", id.0)
    }

    /// Returns every node, in id order.
    pub(crate) fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    /// Returns every stream, in id order.
    pub(crate) fn streams(&self) -> &[Stream] {
        &self.streams
    }

    /// Rejects any stream whose producer did not name it during preflight.
    pub(crate) fn assert_named(&self) -> Result<()> {
        if let Some(stream) = self.streams.iter().find(|stream| stream.name.is_empty()) {
            return Err(WizardError::BuildError(format!(
                "stream {:?} is unnamed",
                stream.id
            )));
        }

        Ok(())
    }

    fn get<'a, T>(items: &'a [T], what: &str, id: usize) -> Result<&'a T> {
        items
            .get(id)
            .ok_or_else(|| WizardError::BuildError(format!("missing {what} {id:?}")))
    }

    fn get_mut<'a, T>(items: &'a mut [T], what: &str, id: usize) -> Result<&'a mut T> {
        items
            .get_mut(id)
            .ok_or_else(|| WizardError::BuildError(format!("missing {what} {id:?}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simple_graph() -> Graph {
        // ARRANGE
        let mut graph = Graph::new();
        let producer = graph.add_node(NodeKind::InstallerPull);
        let consumer = graph.add_node(NodeKind::Concat);
        let stream = graph.add_output(producer, PortId(0)).expect("add output");
        graph
            .bind_input(consumer, PortId(0), stream)
            .expect("bind input");

        graph
    }

    #[test]
    fn binds_and_resolves_ports() {
        // ARRANGE
        let mut graph = Graph::new();
        let producer = graph.add_node(NodeKind::InstallerPull);
        let consumer = graph.add_node(NodeKind::Concat);

        // ACT
        let stream = graph.add_output(producer, PortId(0)).expect("add output");
        graph
            .bind_input(consumer, PortId(0), stream)
            .expect("bind input");

        // ASSERT
        assert_eq!(
            graph
                .node(producer)
                .expect("node")
                .output(PortId(0))
                .expect("output"),
            stream
        );
        assert_eq!(
            graph
                .node(consumer)
                .expect("node")
                .input(PortId(0))
                .expect("input"),
            stream
        );
        assert_eq!(
            graph.stream(stream).expect("stream").consumers,
            vec![consumer]
        );
    }

    #[test]
    fn rejects_missing_port() {
        // ARRANGE
        let mut graph = Graph::new();
        let node = graph.add_node(NodeKind::InstallerPull);
        graph.add_output(node, PortId(0)).expect("add output");

        // ACT
        let missing = graph.node(node).expect("node").output(PortId(1));

        // ASSERT
        missing.unwrap_err();
    }

    #[test]
    fn rejects_duplicate_port() {
        // ARRANGE
        let mut graph = Graph::new();
        let node = graph.add_node(NodeKind::InstallerPull);
        graph.add_output(node, PortId(0)).expect("add output");

        // ACT
        let duplicate = graph.add_output(node, PortId(0));

        // ASSERT
        duplicate.unwrap_err();
    }

    #[test]
    fn rejects_unnamed_streams() {
        // ARRANGE
        let graph = simple_graph();

        // ACT
        let error = graph.assert_named();

        // ASSERT
        assert!(error.is_err(), "unnamed stream must fail assertion");
    }

    #[test]
    fn accepts_named_streams() {
        // ARRANGE
        let mut graph = simple_graph();
        let stream = graph.streams().iter().next().expect("stream").id;
        graph.stream_mut(stream).expect("stream").name = "kernel".to_owned();

        // ACT
        let result = graph.assert_named();

        // ASSERT
        assert!(result.is_ok(), "named stream must pass assertion");
    }

    #[test]
    fn new_streams_start_unstamped() {
        // ARRANGE
        let mut graph = Graph::new();
        let producer = graph.add_node(NodeKind::InstallerPull);

        // ACT
        let stream = graph.add_output(producer, PortId(0)).expect("add output");

        // ASSERT
        assert_eq!(
            graph.stream(stream).expect("stream").artifact,
            None,
            "new streams must not be stamped as terminal artifacts"
        );
    }
}
