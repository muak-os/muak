//! Builds the initial logical graph for a request.

use std::collections::HashMap;

use crate::artifact::Artifact;
use crate::error::{Result, WizardError};
use crate::pipeline::dependency::{
    Dependency, DependencyKind, node_dependencies, output_count, validate,
};
use crate::pipeline::graph::{Graph, NodeId, NodeKind, PortId, StreamId};
use crate::pipeline::normalize::normalize;
use crate::resolve::BuildPlan;

/// Builds the logical DAG for the requested artifacts and normalizes it.
///
/// # Errors
///
/// Returns an error when a dependency is cyclic, a dynamic count cannot be
/// fetched, or the built graph fails validation.
pub(crate) async fn plan(build: &BuildPlan, artifacts: &[Artifact]) -> Result<Graph> {
    let mut planner = Planner::new(build);
    for artifact in artifacts {
        if *artifact == Artifact::Overlays && build.overlay().is_none() {
            return Err(WizardError::BuildError(
                "overlays requested but the profile has no overlay".to_owned(),
            ));
        }
        planner.ensure(NodeKind::ArtifactSink {
            artifact: *artifact,
        })?;
    }
    planner.bind_all().await?;
    validate(&planner.graph, build)?;
    normalize(&mut planner.graph)?;

    Ok(planner.graph)
}

/// Depth-first instantiation of the dependency graph, with memoization.
struct Planner<'a> {
    build: &'a BuildPlan,
    graph: Graph,
    instances: HashMap<NodeKind, NodeId>,
    outputs: HashMap<(NodeKind, PortId), StreamId>,
    counts: HashMap<NodeKind, usize>,
    states: HashMap<NodeKind, VisitState>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum VisitState {
    InProgress,
    Done,
}

impl<'a> Planner<'a> {
    fn new(build: &'a BuildPlan) -> Self {
        Self {
            build,
            graph: Graph::new(),
            instances: HashMap::new(),
            outputs: HashMap::new(),
            counts: HashMap::new(),
            states: HashMap::new(),
        }
    }

    /// Instantiates a node after its producers, so creation is producer-first.
    fn ensure(&mut self, kind: NodeKind) -> Result<NodeId> {
        match self.states.get(&kind).copied() {
            Some(VisitState::Done) => {
                return self.instance(kind);
            }
            Some(VisitState::InProgress) => {
                return Err(WizardError::BuildError(format!(
                    "dependency cycle through {kind:?}"
                )));
            }
            None => {}
        }
        self.states.insert(kind, VisitState::InProgress);

        for dependency in node_dependencies(kind, self.build)? {
            self.ensure(dependency.producer)?;
        }

        let id = self.graph.add_node(kind);
        self.instances.insert(kind, id);
        self.states.insert(kind, VisitState::Done);

        Ok(id)
    }

    /// Binds every node's declared dependencies, in node creation order.
    async fn bind_all(&mut self) -> Result<()> {
        for (consumer, dependency) in self.pending_bindings()? {
            let producer = self.instance(dependency.producer)?;
            self.bind(producer, consumer, &dependency).await?;
        }

        Ok(())
    }

    /// Every `(node, declared dependency)` pair in node creation order.
    fn pending_bindings(&self) -> Result<Vec<(NodeId, Dependency)>> {
        let mut bindings = Vec::new();
        for node in self.graph.nodes() {
            bindings.extend(
                node_dependencies(node.kind, self.build)?
                    .into_iter()
                    .map(|dependency| (node.id, dependency)),
            );
        }

        Ok(bindings)
    }

    fn instance(&self, kind: NodeKind) -> Result<NodeId> {
        self.instances
            .get(&kind)
            .copied()
            .ok_or_else(|| WizardError::BuildError(format!("missing instance for {kind:?}")))
    }

    /// Binds one declared dependency: the producer's output stream (created
    /// on demand, shared with every consumer) to the consumer's input port.
    async fn bind(
        &mut self,
        producer: NodeId,
        consumer: NodeId,
        dependency: &Dependency,
    ) -> Result<()> {
        match dependency.kind {
            DependencyKind::Fixed => {
                let stream =
                    self.output_stream(dependency.producer, dependency.producer_port, producer)?;
                self.graph
                    .bind_input(consumer, dependency.consumer_port, stream)?;
            }
            DependencyKind::Many => self.bind_many(producer, consumer, dependency).await?,
        }

        Ok(())
    }

    /// Binds one stream per dynamic element, in canonical order.
    async fn bind_many(
        &mut self,
        producer: NodeId,
        consumer: NodeId,
        dependency: &Dependency,
    ) -> Result<()> {
        let count = self.dynamic_count(dependency.producer).await?;
        for index in 0..count {
            let producer_port = PortId(dependency.producer_port.0.saturating_add(index));
            let consumer_port = PortId(dependency.consumer_port.0.saturating_add(index));
            let stream = self.output_stream(dependency.producer, producer_port, producer)?;
            self.graph.bind_input(consumer, consumer_port, stream)?;
        }

        Ok(())
    }

    /// The shared stream for a producer output port, created once.
    fn output_stream(
        &mut self,
        kind: NodeKind,
        port: PortId,
        producer: NodeId,
    ) -> Result<StreamId> {
        if let Some(stream) = self.outputs.get(&(kind, port)) {
            return Ok(*stream);
        }
        let stream = self.graph.add_output(producer, port)?;
        self.outputs.insert((kind, port), stream);

        Ok(stream)
    }

    /// The dynamic output count of a `Many` producer, fetched once.
    async fn dynamic_count(&mut self, kind: NodeKind) -> Result<usize> {
        if let Some(count) = self.counts.get(&kind) {
            return Ok(*count);
        }
        let count = output_count(kind, self.build).await?;
        self.counts.insert(kind, count);

        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use koci::arch::Arch;

    use super::*;
    use crate::nodes::uki;
    use crate::request::Platform;
    use crate::source::extension::Extension;

    fn build_plan() -> BuildPlan {
        // ARRANGE
        BuildPlan::new(
            Platform::Metal,
            "v1.0.0".to_owned(),
            Arch::Amd64,
            vec![Extension::new(
                "muak-os/qemu".to_owned(),
                "ghcr.io/muak-os/qemu:v1.0.0".to_owned(),
            )],
            None,
            "ghcr.io/muak-os/installer:v1.0.0".to_owned(),
        )
    }

    fn kinds(graph: &Graph) -> Vec<NodeKind> {
        graph.nodes().iter().map(|node| node.kind).collect()
    }

    fn count(graph: &Graph, kind: NodeKind) -> usize {
        graph
            .nodes()
            .iter()
            .filter(|node| node.kind == kind)
            .count()
    }

    #[tokio::test]
    async fn kernel_and_cmdline_need_only_installer() {
        // ARRANGE
        let build = build_plan();

        // ACT
        let graph = plan(&build, &[Artifact::Kernel, Artifact::Cmdline])
            .await
            .expect("plan");

        // ASSERT
        assert_eq!(kinds(&graph).len(), 3);
        assert_eq!(count(&graph, NodeKind::InstallerPull), 1);
        assert_eq!(
            count(
                &graph,
                NodeKind::ArtifactSink {
                    artifact: Artifact::Kernel
                }
            ),
            1
        );
        assert_eq!(
            count(
                &graph,
                NodeKind::ArtifactSink {
                    artifact: Artifact::Cmdline
                }
            ),
            1
        );
        assert_eq!(count(&graph, NodeKind::Fanout), 0);
    }

    #[tokio::test]
    async fn initramfs_and_uki_fanout_the_complete_initramfs() {
        // ARRANGE
        let build = build_plan();

        // ACT
        let graph = plan(&build, &[Artifact::Initramfs, Artifact::Uki])
            .await
            .expect("plan");

        // ASSERT
        assert_eq!(count(&graph, NodeKind::Uki), 1);
        assert_eq!(count(&graph, NodeKind::Concat), 1);
        assert_eq!(count(&graph, NodeKind::InitramfsTail), 1);
        assert_eq!(count(&graph, NodeKind::ExtensionPayloads), 1);
        assert_eq!(count(&graph, NodeKind::Fanout), 1);
        assert_eq!(
            count(
                &graph,
                NodeKind::ArtifactSink {
                    artifact: Artifact::Initramfs
                }
            ),
            1
        );
        assert_eq!(
            count(
                &graph,
                NodeKind::ArtifactSink {
                    artifact: Artifact::Uki
                }
            ),
            1
        );
    }

    #[tokio::test]
    async fn uki_iso_and_raw_fanout_the_uki_stream() {
        // ARRANGE
        let build = build_plan();

        // ACT
        let graph = plan(&build, &[Artifact::Uki, Artifact::Iso, Artifact::Raw])
            .await
            .expect("plan");

        // ASSERT
        assert_eq!(count(&graph, NodeKind::Uki), 1);
        assert_eq!(count(&graph, NodeKind::Iso), 1);
        assert_eq!(count(&graph, NodeKind::Raw), 1);
        assert_eq!(count(&graph, NodeKind::Fanout), 1);
        assert_eq!(count(&graph, NodeKind::OverlayPull), 0);
    }

    #[tokio::test]
    async fn overlays_without_overlay_profile_rejected() {
        // ARRANGE
        let build = build_plan();

        // ACT
        let error = plan(&build, &[Artifact::Overlays])
            .await
            .expect_err("overlays without overlay must fail");

        // ASSERT
        let message = error.to_string();
        assert!(
            message.contains("overlays requested but the profile has no overlay"),
            "unexpected error: {message}"
        );
    }

    #[tokio::test]
    async fn binds_stable_uki_ports() {
        // ARRANGE
        let build = build_plan();

        // ACT
        let graph = plan(&build, &[Artifact::Uki]).await.expect("plan");

        // ACT
        let uki_node = graph
            .nodes()
            .iter()
            .find(|node| matches!(&node.kind, NodeKind::Uki))
            .expect("uki node");

        // ASSERT
        assert!(uki_node.input(uki::UKI_STUB).is_ok(), "missing stub input");
        assert!(
            uki_node.input(uki::UKI_CMDLINE).is_ok(),
            "missing cmdline input"
        );
        assert!(
            uki_node.input(uki::UKI_KERNEL).is_ok(),
            "missing kernel input"
        );
        assert!(
            uki_node.input(uki::UKI_INITRAMFS).is_ok(),
            "missing initramfs input"
        );
        uki_node.output(uki::UKI_OUTPUT).unwrap();
    }

    #[tokio::test]
    async fn every_artifact_combo_plans_and_validates() {
        // ARRANGE
        let build = build_plan();

        // ACT
        plan(&build, &[Artifact::Kernel, Artifact::Cmdline])
            .await
            .expect("plan");
        plan(&build, &[Artifact::Initramfs, Artifact::Uki])
            .await
            .expect("plan");
        plan(&build, &[Artifact::Uki, Artifact::Iso])
            .await
            .expect("plan");
        plan(&build, &[Artifact::Uki, Artifact::Raw])
            .await
            .expect("plan");

        // ASSERT
    }
}
