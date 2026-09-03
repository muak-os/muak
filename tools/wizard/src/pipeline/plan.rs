//! Builds the initial logical graph for a request.

use std::collections::HashMap;

use crate::artifact::Artifact;
use crate::error::{Result, WizardError};
use crate::nodes::{self, NodeKind};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::graph::Graph;
use crate::pipeline::node::{NodeId, PortId, StreamId};
use crate::pipeline::normalize::normalize;

/// Builds the logical DAG for the requested artifacts and normalizes it.
///
/// # Errors
///
/// Returns an error when an artifact has no unique producer, a dependency is
/// cyclic, or the built graph fails normalization.
pub(crate) fn plan(ctx: &BuildContext<'_, '_>, artifacts: &[Artifact]) -> Result<Graph> {
    let mut planner = Planner::new(ctx);
    for artifact in artifacts {
        if *artifact == Artifact::Overlays && ctx.build.overlay().is_none() {
            return Err(WizardError::BuildError(
                "overlays requested but the profile has no overlay".to_owned(),
            ));
        }
        let (producer, producer_port) = artifact_source(*artifact, ctx)?;
        planner.ensure(producer)?;
        planner.request(producer, producer_port, *artifact)?;
    }
    planner.bind_all()?;
    normalize(&mut planner.graph)?;

    Ok(planner.graph)
}

fn artifact_source(artifact: Artifact, ctx: &BuildContext<'_, '_>) -> Result<(NodeKind, PortId)> {
    let declarations = NodeKind::ALL
        .iter()
        .copied()
        .flat_map(|kind| {
            nodes::produces(kind, ctx)
                .into_iter()
                .map(move |(port, produced)| (kind, port, produced))
        })
        .filter(|&(.., produced)| produced == artifact);
    let mut source = None;
    for (kind, port, _) in declarations {
        if source.is_some() {
            return Err(WizardError::BuildError(format!(
                "artifact {artifact} is produced by more than one node"
            )));
        }
        source = Some((kind, port));
    }

    source.ok_or_else(|| WizardError::BuildError(format!("no node produces {artifact}")))
}

/// Depth-first instantiation of the dependency graph, with memoization.
struct Planner<'a, 'data, 'sign> {
    ctx: &'a BuildContext<'data, 'sign>,
    graph: Graph,
    instances: HashMap<NodeKind, NodeId>,
    outputs: HashMap<(NodeKind, PortId), StreamId>,
    states: HashMap<NodeKind, VisitState>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum VisitState {
    InProgress,
    Done,
}

impl<'a, 'data, 'sign> Planner<'a, 'data, 'sign> {
    fn new(ctx: &'a BuildContext<'data, 'sign>) -> Self {
        Self {
            ctx,
            graph: Graph::new(),
            instances: HashMap::new(),
            outputs: HashMap::new(),
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

        for dependency in nodes::dependencies(kind, self.ctx) {
            self.ensure(dependency.producer)?;
        }

        let id = self.graph.add_node(kind);
        self.instances.insert(kind, id);
        self.states.insert(kind, VisitState::Done);

        Ok(id)
    }

    /// Binds every node's declared dependencies, in node creation order.
    fn bind_all(&mut self) -> Result<()> {
        for (consumer, dependency) in self.pending_bindings() {
            let producer = self.instance(dependency.producer)?;
            self.bind(producer, consumer, &dependency)?;
        }

        Ok(())
    }

    /// Stamps a producer output as the terminal stream of a requested artifact.
    fn request(&mut self, kind: NodeKind, port: PortId, artifact: Artifact) -> Result<()> {
        let producer = self.instance(kind)?;
        let stream = self.output_stream(kind, port, producer)?;
        self.graph.stream_mut(stream)?.artifact = Some(artifact);

        Ok(())
    }

    /// Every `(node, declared dependency)` pair in node creation order.
    fn pending_bindings(&self) -> Vec<(NodeId, Dependency)> {
        let mut bindings = Vec::new();
        for node in self.graph.nodes() {
            bindings.extend(
                nodes::dependencies(node.kind, self.ctx)
                    .into_iter()
                    .map(|dependency| (node.id, dependency)),
            );
        }

        bindings
    }

    fn instance(&self, kind: NodeKind) -> Result<NodeId> {
        self.instances
            .get(&kind)
            .copied()
            .ok_or_else(|| WizardError::BuildError(format!("missing instance for {kind:?}")))
    }

    /// Binds one declared dependency: the producer's output stream (created
    /// on demand, shared with every consumer) to the consumer's input port.
    fn bind(&mut self, producer: NodeId, consumer: NodeId, dependency: &Dependency) -> Result<()> {
        let stream = self.output_stream(dependency.producer, dependency.producer_port, producer)?;
        self.graph
            .bind_input(consumer, dependency.consumer_port, stream)
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
}

#[cfg(test)]
mod tests {
    use koci::arch::Arch;
    use sbolt::keys::SigningPair;
    use sbolt::keys::cert::generate_pk;

    use super::*;
    use crate::domain::resolution::Extension;
    use crate::domain::resolution::Kernel;
    use crate::domain::resolution::Overlay;
    use crate::domain::resolution::ResolvedBuild;
    use crate::nodes::kernel;
    use crate::nodes::uki;
    use crate::pipeline::context::BuildContext;
    use crate::request::Platform;

    fn build_plan() -> ResolvedBuild {
        ResolvedBuild::new(
            Platform::Metal,
            "v1.0.0".to_owned(),
            Arch::Amd64,
            Kernel::new(
                "ghcr.io/muak-os/linux".to_owned(),
                "ghcr.io/muak-os/linux:v1.0.0".to_owned(),
            ),
        )
        .with_sources(
            "ghcr.io/muak-os/stub:v1.0.0".to_owned(),
            "ghcr.io/muak-os/installer:v1.0.0".to_owned(),
            None,
            vec![Extension::new(
                "muak-os/qemu".to_owned(),
                "ghcr.io/muak-os/qemu:v1.0.0".to_owned(),
            )],
        )
    }

    fn build_plan_with_overlay() -> ResolvedBuild {
        ResolvedBuild::new(
            Platform::Metal,
            "v1.0.0".to_owned(),
            Arch::Amd64,
            Kernel::new(
                "ghcr.io/muak-os/linux".to_owned(),
                "ghcr.io/muak-os/linux:v1.0.0".to_owned(),
            ),
        )
        .with_sources(
            "ghcr.io/muak-os/stub:v1.0.0".to_owned(),
            "ghcr.io/muak-os/installer:v1.0.0".to_owned(),
            Some(Overlay::new(
                "muak".to_owned(),
                "muak-os/overlays".to_owned(),
                "ghcr.io/muak-os/overlays:v1.0.0".to_owned(),
                Arch::Amd64,
            )),
            Vec::new(),
        )
    }

    fn context(build: &ResolvedBuild) -> BuildContext<'_, '_> {
        BuildContext {
            build,
            profile: b"",
            signing: None,
        }
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

    fn terminal(graph: &Graph, artifact: Artifact) -> (NodeKind, PortId) {
        let stream = graph
            .streams()
            .iter()
            .find(|stream| stream.artifact == Some(artifact))
            .unwrap_or_else(|| panic!("no terminal stream for {artifact}"));
        let producer = graph.node(stream.producer).expect("producer").kind;

        (
            producer,
            graph
                .node(stream.producer)
                .expect("node")
                .outputs
                .iter()
                .find(|binding| binding.stream == stream.id)
                .expect("terminal port")
                .port,
        )
    }

    fn terminals(graph: &Graph) -> Vec<Artifact> {
        let mut artifacts: Vec<_> = graph
            .streams()
            .iter()
            .filter_map(|stream| stream.artifact)
            .collect();
        artifacts.sort();

        artifacts
    }

    #[test]
    fn kernel_and_cmdline_need_only_kernel_pull() {
        // ARRANGE
        let build = build_plan();
        let ctx = context(&build);

        // ACT
        let graph = plan(&ctx, &[Artifact::Kernel, Artifact::Cmdline]).expect("plan");

        // ASSERT
        assert_eq!(kinds(&graph), vec![NodeKind::KernelPull]);
        assert_eq!(
            terminal(&graph, Artifact::Kernel),
            (NodeKind::KernelPull, kernel::KERNEL)
        );
        assert_eq!(
            terminal(&graph, Artifact::Cmdline),
            (NodeKind::KernelPull, kernel::CMDLINE)
        );
        assert_eq!(count(&graph, NodeKind::Fanout), 0);
    }

    #[test]
    fn initramfs_and_uki_fanout_the_complete_initramfs() {
        // ARRANGE
        let build = build_plan();
        let ctx = context(&build);

        // ACT
        let graph = plan(&ctx, &[Artifact::Initramfs, Artifact::Uki]).expect("plan");

        // ASSERT
        assert_eq!(count(&graph, NodeKind::Uki), 1);
        assert_eq!(count(&graph, NodeKind::Concat), 1);
        assert_eq!(count(&graph, NodeKind::InitramfsTail), 1);
        assert_eq!(count(&graph, NodeKind::LayerPayloads), 1);
        assert_eq!(count(&graph, NodeKind::Fanout), 1);
        assert_eq!(
            terminal(&graph, Artifact::Initramfs).0,
            NodeKind::Fanout,
            "the shared initramfs stream must end in a stamped fanout branch"
        );
        assert_eq!(terminal(&graph, Artifact::Uki).0, NodeKind::Uki);
    }

    #[test]
    fn uki_iso_and_raw_fanout_the_uki_stream() {
        // ARRANGE
        let build = build_plan();
        let ctx = context(&build);

        // ACT
        let graph = plan(&ctx, &[Artifact::Uki, Artifact::Iso, Artifact::Raw]).expect("plan");

        // ASSERT
        assert_eq!(count(&graph, NodeKind::Uki), 1);
        assert_eq!(count(&graph, NodeKind::Iso), 1);
        assert_eq!(count(&graph, NodeKind::Raw), 1);
        assert_eq!(count(&graph, NodeKind::Fanout), 1);
        assert_eq!(count(&graph, NodeKind::OverlayPull), 0);
    }

    #[test]
    fn overlays_without_overlay_profile_rejected() {
        // ARRANGE
        let build = build_plan();
        let ctx = context(&build);

        // ACT
        let error =
            plan(&ctx, &[Artifact::Overlays]).expect_err("overlays without overlay must fail");

        // ASSERT
        let message = error.to_string();
        assert!(
            message.contains("overlays requested but the profile has no overlay"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn signed_request_routes_the_uki_through_sign() {
        // ARRANGE
        let build = build_plan();
        let (signer, certificate) = generate_pk("muak-test").expect("generate signing pair");
        let signing = SigningPair {
            signer: &signer,
            certificate: &certificate,
        };
        let ctx = BuildContext {
            build: &build,
            profile: b"",
            signing: Some(&signing),
        };

        // ACT
        let graph = plan(&ctx, &[Artifact::Uki]).expect("plan");

        // ASSERT
        assert_eq!(count(&graph, NodeKind::Sign), 1);
        assert_eq!(count(&graph, NodeKind::Uki), 1);
        assert_eq!(terminal(&graph, Artifact::Uki).0, NodeKind::Sign);
    }

    #[test]
    fn binds_stable_uki_ports() {
        // ARRANGE
        let build = build_plan();
        let ctx = context(&build);

        // ACT
        let graph = plan(&ctx, &[Artifact::Uki]).expect("plan");

        // ASSERT
        assert_eq!(count(&graph, NodeKind::Sign), 0);

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

    #[test]
    fn every_artifact_combo_plans_and_normalizes() {
        // ARRANGE
        let build = build_plan();
        let ctx = context(&build);

        // ACT
        plan(&ctx, &[Artifact::Kernel, Artifact::Cmdline]).expect("plan");
        plan(&ctx, &[Artifact::Initramfs, Artifact::Uki]).expect("plan");
        plan(&ctx, &[Artifact::Uki, Artifact::Iso]).expect("plan");
        plan(&ctx, &[Artifact::Uki, Artifact::Raw]).expect("plan");

        // ASSERT
    }

    #[test]
    fn every_artifact_plans_with_a_unique_terminal_stream() {
        // ARRANGE
        let build = build_plan_with_overlay();
        let artifacts = [
            Artifact::Kernel,
            Artifact::Initramfs,
            Artifact::Cmdline,
            Artifact::Uki,
            Artifact::Iso,
            Artifact::Raw,
            Artifact::Overlays,
        ];
        let ctx = context(&build);

        // ACT
        let graph = plan(&ctx, &artifacts).expect("plan");

        // ASSERT
        let mut stamped = terminals(&graph);
        stamped.sort_unstable();
        let mut requested = artifacts.to_vec();
        requested.sort_unstable();
        assert_eq!(
            stamped, requested,
            "every artifact needs one terminal stream"
        );
    }

    #[test]
    fn duplicate_artifacts_dedup_into_one_terminal_stream() {
        // ARRANGE
        let build = build_plan();
        let ctx = context(&build);

        // ACT
        let graph = plan(&ctx, &[Artifact::Kernel, Artifact::Kernel]).expect("plan");

        // ASSERT
        assert_eq!(terminals(&graph), vec![Artifact::Kernel]);
    }
}
