//! Node dependency declarations, their interpretation, and validation.

use crate::error::{Result, WizardError};
use crate::nodes::{self, NodeKind};
use crate::pipeline::context::BuildContext;
use crate::pipeline::graph::{Graph, Node, PortId};

/// One declared input of a node: the stream produced by `producer` on
/// `producer_port`, bound to the consumer's `consumer_port`.
///
/// Producers with dynamic arity are consumed through multiple declared
/// dependencies, one per positional port, so every dependency is exactly
/// one stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Dependency {
    pub(crate) producer: NodeKind,
    pub(crate) producer_port: PortId,
    pub(crate) consumer_port: PortId,
}

impl Dependency {
    /// One stream between the two ports.
    #[must_use]
    pub(crate) const fn new(
        producer: NodeKind,
        producer_port: PortId,
        consumer_port: PortId,
    ) -> Self {
        Self {
            producer,
            producer_port,
            consumer_port,
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
        check(producer, node, &dependency)?;
    }

    Ok(())
}

/// Checks the declared stream binds the producer's output to the consumer's input.
fn check(producer: &Node, node: &Node, dependency: &Dependency) -> Result<()> {
    let Some(output) = producer
        .outputs
        .iter()
        .find(|binding| binding.port == dependency.producer_port)
    else {
        return Err(WizardError::BuildError(format!(
            "node {:?} dependency on {:?} has no producer output at port {:?}",
            node.kind, dependency.producer, dependency.producer_port,
        )));
    };
    let Some(input) = node
        .inputs
        .iter()
        .find(|binding| binding.port == dependency.consumer_port)
    else {
        return Err(WizardError::BuildError(format!(
            "node {:?} has no input at port {:?} for its dependency on {:?}",
            node.kind, dependency.consumer_port, dependency.producer,
        )));
    };

    if output.stream != input.stream {
        return Err(WizardError::BuildError(format!(
            "node {:?} dependency on {:?} port {:?} binds stream {input:?}, producer has {output:?}",
            node.kind, dependency.producer, dependency.producer_port,
        )));
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

#[cfg(test)]
mod tests {
    use koci::arch::Arch;

    use crate::artifact::Artifact;
    use crate::domain::overlay::Asset;
    use crate::domain::resolution::Kernel;
    use crate::domain::resolution::Overlay;
    use crate::domain::resolution::{ResolvedBuild, Sources};
    use crate::nodes::NodeKind;
    use crate::nodes::kernel;
    use crate::pipeline::context::BuildContext;
    use crate::pipeline::dependency::validate;
    use crate::pipeline::graph::{Graph, PortId};
    use crate::request::Platform;

    fn build_plan() -> ResolvedBuild {
        ResolvedBuild::new(
            Platform::Metal,
            "v1.0.0".to_owned(),
            Arch::Amd64,
            Sources {
                stub: "ghcr.io/muak-os/stub:v1.0.0".to_owned(),
                installer: "ghcr.io/muak-os/installer:v1.0.0".to_owned(),
                kernel: Kernel::new(
                    "ghcr.io/muak-os/linux".to_owned(),
                    "ghcr.io/muak-os/linux:v1.0.0".to_owned(),
                ),
                overlay: None,
                extensions: Vec::new(),
            },
        )
    }

    fn context(build: &ResolvedBuild) -> BuildContext<'_, '_, '_> {
        BuildContext {
            build,
            profile: b"",
            signing: None,
            writers: std::sync::Mutex::new(
                crate::pipeline::context::TargetWriters::new(Vec::new()),
            ),
        }
    }

    fn build_plan_with_assets(assets: Vec<Asset>) -> ResolvedBuild {
        ResolvedBuild::new(
            Platform::Metal,
            "v1.0.0".to_owned(),
            Arch::Amd64,
            Sources {
                stub: "ghcr.io/muak-os/stub:v1.0.0".to_owned(),
                installer: "ghcr.io/muak-os/installer:v1.0.0".to_owned(),
                kernel: Kernel::new(
                    "ghcr.io/muak-os/linux".to_owned(),
                    "ghcr.io/muak-os/linux:v1.0.0".to_owned(),
                ),
                overlay: Some(Overlay::new(
                    "board".to_owned(),
                    "board".to_owned(),
                    "ghcr.io/example/board:latest".to_owned(),
                    Arch::Arm64,
                )),
                extensions: Vec::new(),
            },
        )
        .with_overlay_assets(Some(assets))
    }

    fn esp_file(path: &str) -> Asset {
        Asset::EspFile {
            path: path.to_owned(),
            size: 1,
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
        let build = build_plan();
        let ctx = context(&build);

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
        let build = build_plan();
        let ctx = context(&build);

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
        let other = graph.add_node(NodeKind::InstallerPull);
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
        let build = build_plan();
        let ctx = context(&build);

        // ACT
        let error = validate(&graph, &ctx).expect_err("stream mismatch");

        // ASSERT
        let message = error.to_string();
        assert!(
            message.contains("binds stream"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn rejects_many_count_mismatch() {
        // ARRANGE
        let build = build_plan_with_assets(vec![esp_file("config.txt"), esp_file("boot.efi")]);
        let mut graph = Graph::new();
        let pull = graph.add_node(NodeKind::OverlayPull);
        let tar = graph.add_node(NodeKind::OverlayTar);
        let stream = graph.add_output(pull, PortId(0)).expect("add output");
        graph.bind_input(tar, PortId(1), stream).expect("bind");
        graph.bind_input(tar, PortId(2), stream).expect("bind");
        let ctx = context(&build);

        // ACT
        let error = validate(&graph, &ctx).expect_err("count mismatch");

        // ASSERT
        let message = error.to_string();
        assert!(
            message.contains("has no producer output at port"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn rejects_many_stream_mismatch() {
        // ARRANGE
        let build = build_plan_with_assets(vec![esp_file("config.txt"), esp_file("boot.efi")]);
        let mut graph = Graph::new();
        let pull = graph.add_node(NodeKind::OverlayPull);
        let other = graph.add_node(NodeKind::InstallerPull);
        let tar = graph.add_node(NodeKind::OverlayTar);
        let first = graph.add_output(pull, PortId(0)).expect("add output");
        let _second = graph.add_output(pull, PortId(1)).expect("add output");
        let wrong = graph.add_output(other, PortId(0)).expect("add output");
        graph.bind_input(tar, PortId(1), first).expect("bind");
        graph.bind_input(tar, PortId(2), wrong).expect("bind");
        let ctx = context(&build);

        // ACT
        let error = validate(&graph, &ctx).expect_err("stream mismatch");

        // ASSERT
        let message = error.to_string();
        assert!(
            message.contains("binds stream"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn rejects_duplicate_node_instances() {
        // ARRANGE
        let mut graph = kernel_sink_graph();
        graph.add_node(NodeKind::KernelPull);
        let build = build_plan();
        let ctx = context(&build);

        // ACT
        let result = validate(&graph, &ctx);

        // ASSERT
        result.unwrap_err();
    }
}
