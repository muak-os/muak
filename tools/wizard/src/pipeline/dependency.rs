//! Node dependency declarations, their interpretation, and validation.

use crate::error::{Result, WizardError};
use crate::nodes::{self, NodeKind};
use crate::pipeline::context::BuildContext;
use crate::pipeline::graph::{Graph, Node, PortBinding, PortId};

/// One declared input of a node: a stream produced by `producer` on
/// `producer_port`, bound to the consumer's `consumer_port`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Dependency {
    pub(crate) producer: NodeKind,
    pub(crate) producer_port: PortId,
    pub(crate) consumer_port: PortId,
    pub(crate) kind: DependencyKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DependencyKind {
    /// Exactly one stream between the two ports.
    Fixed,
    /// One stream per element of the producer's dynamic output range.
    Many,
}

impl Dependency {
    /// Exactly one stream between the two ports.
    #[must_use]
    pub(crate) fn fixed(producer: NodeKind, producer_port: PortId, consumer_port: PortId) -> Self {
        Self {
            producer,
            producer_port,
            consumer_port,
            kind: DependencyKind::Fixed,
        }
    }

    /// One stream per dynamic element; the planner binds
    /// `producer_first + i` to `consumer_first + i` for every `i`.
    #[must_use]
    pub(crate) fn many(producer: NodeKind, producer_first: PortId, consumer_first: PortId) -> Self {
        Self {
            producer,
            producer_port: producer_first,
            consumer_port: consumer_first,
            kind: DependencyKind::Many,
        }
    }
}

/// Checks every instantiated node's declared dependencies are bound.
///
/// # Errors
///
/// Returns an error on the first violation.
pub(crate) fn validate(graph: &Graph, ctx: &BuildContext<'_, '_, '_>) -> Result<()> {
    for node in graph.nodes() {
        check_node_dependencies(graph, node, ctx)?;
    }
    for node in graph.nodes() {
        let instances = graph
            .nodes()
            .iter()
            .filter(|other| other.kind == node.kind)
            .count();
        if instances != 1 {
            return Err(WizardError::BuildError(format!(
                "node kind {:?} has {instances} instances, expected 1",
                node.kind
            )));
        }
    }

    Ok(())
}

fn check_node_dependencies(
    graph: &Graph,
    node: &Node,
    ctx: &BuildContext<'_, '_, '_>,
) -> Result<()> {
    for dependency in nodes::dependencies(node.kind, ctx) {
        let producer = find_node(graph, dependency.producer)?;
        match dependency.kind {
            DependencyKind::Fixed => check_fixed(producer, node, &dependency)?,
            DependencyKind::Many => check_many(producer, node, &dependency)?,
        }
    }

    Ok(())
}

fn check_fixed(producer: &Node, node: &Node, dependency: &Dependency) -> Result<()> {
    let output = producer.output(dependency.producer_port)?;
    let input = node.input(dependency.consumer_port)?;

    if output != input {
        return Err(WizardError::BuildError(format!(
            "node {:?} dependency on {:?} port {:?} binds stream {input:?}, producer has {output:?}",
            node.kind, dependency.producer, dependency.producer_port,
        )));
    }

    Ok(())
}

fn check_many(producer: &Node, node: &Node, dependency: &Dependency) -> Result<()> {
    let outputs = dynamic_bindings(&producer.outputs, dependency.producer_port);
    let inputs = dynamic_bindings(&node.inputs, dependency.consumer_port);

    if outputs.len() != inputs.len() {
        return Err(WizardError::BuildError(format!(
            "node {:?} dependency on {:?} count mismatch: {} != {}",
            node.kind,
            dependency.producer,
            outputs.len(),
            inputs.len(),
        )));
    }

    for (output, input) in outputs.iter().zip(&inputs) {
        if output.stream != input.stream {
            return Err(WizardError::BuildError(format!(
                "node {:?} dynamic dependency on {:?} stream mismatch: {:?} != {:?}",
                node.kind, dependency.producer, output.stream, input.stream,
            )));
        }
    }

    Ok(())
}

fn find_node(graph: &Graph, kind: NodeKind) -> Result<&Node> {
    graph
        .nodes()
        .iter()
        .find(|node| node.kind == kind)
        .ok_or_else(|| WizardError::BuildError(format!("missing producer node {kind:?}")))
}

/// Every binding at or after `first`, in planner (ascending) order.
fn dynamic_bindings(bindings: &[PortBinding], first: PortId) -> Vec<&PortBinding> {
    bindings
        .iter()
        .filter(|binding| binding.port >= first)
        .collect()
}

#[cfg(test)]
mod tests {
    use koci::arch::Arch;

    use crate::artifact::Artifact;
    use crate::nodes::NodeKind;
    use crate::nodes::kernel;
    use crate::pipeline::context::BuildContext;
    use crate::pipeline::dependency::validate;
    use crate::pipeline::graph::{Graph, PortId};
    use crate::request::Platform;
    use crate::resolve::BuildPlan;
    use crate::source::kernel::Kernel;

    fn build_plan() -> BuildPlan {
        BuildPlan::new(
            Platform::Metal,
            "v1.0.0".to_owned(),
            Arch::Amd64,
            Vec::new(),
            None,
            Kernel::new(
                "ghcr.io/muak-os/kernel".to_owned(),
                "ghcr.io/muak-os/kernel:v1.0.0".to_owned(),
            ),
            "ghcr.io/muak-os/installer:v1.0.0".to_owned(),
        )
    }

    fn context(plan: &BuildPlan) -> BuildContext<'_, '_, '_> {
        BuildContext {
            plan,
            profile: b"",
            signing: None,
            writers: std::sync::Mutex::new(
                crate::pipeline::context::TargetWriters::new(Vec::new()),
            ),
        }
    }

    fn kernel_sink_graph() -> Graph {
        // ARRANGE
        let mut graph = Graph::new();
        let producer = graph.add_node(NodeKind::KernelPull);
        let consumer = graph.add_node(NodeKind::ArtifactSink {
            artifact: Artifact::Kernel,
        });
        let stream = graph
            .add_output(producer, kernel::KERNEL)
            .expect("add output");
        graph.bind_input(consumer, PortId(0), stream).expect("bind");

        graph
    }

    #[test]
    fn accepts_satisfied_dependencies() {
        // ARRANGE
        let graph = kernel_sink_graph();
        let plan = build_plan();
        let ctx = context(&plan);

        // ACT
        let result = validate(&graph, &ctx);

        // ASSERT
        result.unwrap();
    }

    #[test]
    fn rejects_missing_producer() {
        // ARRANGE
        let mut graph = Graph::new();
        graph.add_node(NodeKind::ArtifactSink {
            artifact: Artifact::Kernel,
        });
        let plan = build_plan();
        let ctx = context(&plan);

        // ACT
        let result = validate(&graph, &ctx);

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn rejects_fixed_stream_mismatch() {
        // ARRANGE
        let mut graph = Graph::new();
        let producer = graph.add_node(NodeKind::KernelPull);
        let other = graph.add_node(NodeKind::Concat);
        let consumer = graph.add_node(NodeKind::ArtifactSink {
            artifact: Artifact::Kernel,
        });
        graph
            .add_output(producer, kernel::KERNEL)
            .expect("add output");
        let other_stream = graph.add_output(other, PortId(0)).expect("add output");
        graph
            .bind_input(consumer, PortId(0), other_stream)
            .expect("bind");
        let plan = build_plan();
        let ctx = context(&plan);

        // ACT
        let result = validate(&graph, &ctx);

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn rejects_many_count_mismatch() {
        // ARRANGE
        let mut graph = Graph::new();
        let pull = graph.add_node(NodeKind::OverlayPull);
        let tar = graph.add_node(NodeKind::OverlayTar);
        let stream = graph.add_output(pull, PortId(0)).expect("add output");
        graph.bind_input(tar, PortId(1), stream).expect("bind");
        graph.bind_input(tar, PortId(2), stream).expect("bind");
        let plan = build_plan();
        let ctx = context(&plan);

        // ACT
        let result = validate(&graph, &ctx);

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn rejects_many_stream_mismatch() {
        // ARRANGE
        let mut graph = Graph::new();
        let pull = graph.add_node(NodeKind::OverlayPull);
        let other = graph.add_node(NodeKind::Concat);
        let tar = graph.add_node(NodeKind::OverlayTar);
        let first = graph.add_output(pull, PortId(0)).expect("add output");
        let _second = graph.add_output(pull, PortId(1)).expect("add output");
        let wrong = graph.add_output(other, PortId(0)).expect("add output");
        graph.bind_input(tar, PortId(1), first).expect("bind");
        graph.bind_input(tar, PortId(2), wrong).expect("bind");
        let plan = build_plan();
        let ctx = context(&plan);

        // ACT
        let result = validate(&graph, &ctx);

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn rejects_duplicate_node_instances() {
        // ARRANGE
        let mut graph = kernel_sink_graph();
        graph.add_node(NodeKind::KernelPull);
        let plan = build_plan();
        let ctx = context(&plan);

        // ACT
        let result = validate(&graph, &ctx);

        // ASSERT
        result.unwrap_err();
    }
}
