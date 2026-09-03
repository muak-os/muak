//! Logical build graph types.

use crate::error::{Result, WizardError};
use crate::nodes::NodeKind;
use crate::pipeline::order;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct NodeId(pub(crate) usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct StreamId(pub(crate) usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct PortId(pub(crate) usize);

impl PortId {
    /// The port `index` positions after this one, saturating.
    #[must_use]
    pub(crate) const fn offset(self, index: usize) -> Self {
        Self(self.0.saturating_add(index))
    }
}

/// Node-local port identity paired with the logical stream it carries.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PortBinding {
    pub(crate) port: PortId,
    pub(crate) stream: StreamId,
}

/// A logical operation consuming and/or producing streams.
#[derive(Clone, Debug)]
pub(crate) struct Node {
    pub(crate) id: NodeId,
    pub(crate) kind: NodeKind,
    pub(crate) inputs: Vec<PortBinding>,
    pub(crate) outputs: Vec<PortBinding>,
}

impl Node {
    /// Stream bound to a fixed input port.
    pub(crate) fn input(&self, port: PortId) -> Result<StreamId> {
        self.inputs
            .iter()
            .find(|binding| binding.port == port)
            .map(|binding| binding.stream)
            .ok_or_else(|| {
                WizardError::BuildError(format!("node {:?} has no input port {port:?}", self.id))
            })
    }

    /// Stream bound to a fixed output port.
    pub(crate) fn output(&self, port: PortId) -> Result<StreamId> {
        self.outputs
            .iter()
            .find(|binding| binding.port == port)
            .map(|binding| binding.stream)
            .ok_or_else(|| {
                WizardError::BuildError(format!("node {:?} has no output port {port:?}", self.id))
            })
    }

    /// Every port-to-stream input binding, for generic binding and dynamic ports.
    pub(crate) fn input_bindings(&self) -> impl Iterator<Item = &PortBinding> {
        self.inputs.iter()
    }

    /// Every port-to-stream output binding, for generic binding and dynamic ports.
    pub(crate) fn output_bindings(&self) -> impl Iterator<Item = &PortBinding> {
        self.outputs.iter()
    }
}

/// A logical byte flow with a fixed size, one producer, and one consumer.
#[derive(Clone, Debug)]
pub(crate) struct Stream {
    pub(crate) id: StreamId,
    pub(crate) name: String,
    pub(crate) size: u64,
    pub(crate) producer: NodeId,
    pub(crate) consumers: Vec<NodeId>,
}

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
            producer: node,
            consumers: Vec::new(),
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

    /// Rebinds an input port of `node` from `old` to `new` (normalization).
    pub(crate) fn rebind_input(
        &mut self,
        node: NodeId,
        old: StreamId,
        new: StreamId,
    ) -> Result<()> {
        let binding = self
            .node_mut(node)?
            .inputs
            .iter_mut()
            .find(|binding| binding.stream == old)
            .ok_or_else(|| {
                WizardError::BuildError(format!("node {node:?} has no input stream {old:?}"))
            })?;
        binding.stream = new;
        self.stream_mut(old)?
            .consumers
            .retain(|consumer| *consumer != node);
        self.stream_mut(new)?.consumers.push(node);

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

    /// Deterministic producer-before-consumer order (Kahn's algorithm).
    #[must_use]
    pub(crate) fn topological_order(&self) -> Vec<NodeId> {
        order::topological(&self.nodes, &self.streams)
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
    use crate::artifact::Artifact;

    fn simple_graph() -> Graph {
        // ARRANGE
        let mut graph = Graph::new();
        let producer = graph.add_node(NodeKind::InstallerPull);
        let consumer = graph.add_node(NodeKind::ArtifactSink {
            artifact: Artifact::Kernel,
        });
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
        let consumer = graph.add_node(NodeKind::ArtifactSink {
            artifact: Artifact::Kernel,
        });

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
        assert_eq!(graph.stream(stream).expect("stream").producer, producer);
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
    fn rebinds_input_stream() {
        // ARRANGE
        let mut graph = simple_graph();
        let stream = graph.streams().iter().next().expect("stream").id;
        let producer = graph.stream(stream).expect("stream").producer;
        let consumer = graph
            .stream(stream)
            .expect("stream")
            .consumers
            .first()
            .copied()
            .expect("consumer");
        let replacement = graph.add_output(producer, PortId(1)).expect("add output");

        // ACT
        graph
            .rebind_input(consumer, stream, replacement)
            .expect("rebind");

        // ASSERT
        assert_eq!(graph.stream(stream).expect("stream").consumers, Vec::new());
        assert_eq!(
            graph.stream(replacement).expect("stream").consumers,
            vec![consumer]
        );
    }

    #[test]
    fn topological_order_places_producers_first() {
        // ARRANGE
        let mut graph = Graph::new();
        let installer_node = graph.add_node(NodeKind::InstallerPull);
        let tail = graph.add_node(NodeKind::InitramfsTail);
        let concat = graph.add_node(NodeKind::Concat);
        let base = graph
            .add_output(installer_node, PortId(0))
            .expect("add output");
        let tail_stream = graph.add_output(tail, PortId(0)).expect("add output");
        graph.bind_input(concat, PortId(0), base).expect("bind");
        graph
            .bind_input(concat, PortId(1), tail_stream)
            .expect("bind");
        graph.add_output(concat, PortId(2)).expect("add output");

        // ACT
        let order = graph.topological_order();

        // ASSERT
        let index = |id: NodeId| order.iter().position(|item| *item == id).expect("in order");
        assert!(index(installer_node) < index(concat));
        assert!(index(tail) < index(concat));
        assert_eq!(order.len(), 3);
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
}
