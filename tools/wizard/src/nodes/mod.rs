//! Build pipeline node runners.

pub(crate) mod fanout;
pub(crate) mod initramfs;
pub(crate) mod installer;
pub(crate) mod kernel;
pub(crate) mod layers;
pub(crate) mod media;
pub(crate) mod overlay;
pub(crate) mod sign;
pub(crate) mod stub;
pub(crate) mod uki;

use crate::artifact::Artifact;
use crate::error::Result;
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::{Graph, NodeId, PortId};
use crate::pipeline::runtime::NodePorts;

/// What a node does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum NodeKind {
    InstallerPull,
    StubPull,
    KernelPull,
    LayerPayloads,
    InitramfsTail,
    Concat,
    Uki,
    Sign,
    Iso,
    Raw,
    OverlayPull,
    OverlayTar,
    Fanout,
}

impl NodeKind {
    /// Every node kind, for produce-table scans. Keep in sync with the enum.
    pub(crate) const ALL: [NodeKind; 12] = [
        Self::InstallerPull,
        Self::StubPull,
        Self::KernelPull,
        Self::LayerPayloads,
        Self::InitramfsTail,
        Self::Concat,
        Self::Uki,
        Self::Sign,
        Self::Iso,
        Self::Raw,
        Self::OverlayPull,
        Self::OverlayTar,
    ];
}

/// One node kind's full contract.
pub(crate) struct NodeDescriptor {
    pub(crate) dependencies: fn(NodeKind, &BuildContext<'_, '_>) -> Vec<Dependency>,
    pub(crate) produces: fn(NodeKind, &BuildContext<'_, '_>) -> Vec<(PortId, Artifact)>,
    pub(crate) preflight: fn(&mut Graph, NodeId, &BuildContext<'_, '_>) -> Result<()>,
    pub(crate) run:
        fn(NodeKind, &mut NodePorts<'_, '_>, &BuildContext<'_, '_>) -> Result<NodeReport>,
}

/// The kind → descriptor catalog.
pub(crate) fn descriptor(kind: NodeKind) -> &'static NodeDescriptor {
    match kind {
        NodeKind::InstallerPull => &installer::DESCRIPTOR,
        NodeKind::StubPull => &stub::DESCRIPTOR,
        NodeKind::KernelPull => &kernel::DESCRIPTOR,
        NodeKind::LayerPayloads => &layers::DESCRIPTOR,
        NodeKind::InitramfsTail => &initramfs::tail::DESCRIPTOR,
        NodeKind::Concat => &initramfs::concat::DESCRIPTOR,
        NodeKind::Uki => &uki::DESCRIPTOR,
        NodeKind::Sign => &sign::DESCRIPTOR,
        NodeKind::Iso => &media::iso::DESCRIPTOR,
        NodeKind::Raw => &media::raw::DESCRIPTOR,
        NodeKind::OverlayPull => &overlay::pull::DESCRIPTOR,
        NodeKind::OverlayTar => &overlay::tar::DESCRIPTOR,
        NodeKind::Fanout => &fanout::DESCRIPTOR,
    }
}

/// Declared inputs of a planned kind, read from its descriptor.
pub(crate) fn dependencies(kind: NodeKind, ctx: &BuildContext<'_, '_>) -> Vec<Dependency> {
    (descriptor(kind).dependencies)(kind, ctx)
}

/// Declared artifact outputs of a planned kind: which port yields which artifact.
pub(crate) fn produces(kind: NodeKind, ctx: &BuildContext<'_, '_>) -> Vec<(PortId, Artifact)> {
    (descriptor(kind).produces)(kind, ctx)
}

#[cfg(test)]
mod tests {
    use koci::arch::Arch;
    use sbolt::keys::SigningPair;
    use sbolt::keys::cert::generate_pk;

    use super::*;
    use crate::domain::resolution::Kernel;
    use crate::domain::resolution::{ResolvedBuild, Sources};
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

    fn context(build: &ResolvedBuild) -> BuildContext<'_, '_> {
        BuildContext {
            build,
            profile: b"",
            signing: None,
        }
    }

    #[test]
    fn produces_declares_artifact_outputs_through_the_descriptor_table() {
        // ARRANGE
        let build = build_plan();
        let ctx = context(&build);
        let routes = [
            (NodeKind::KernelPull, (kernel::KERNEL, Artifact::Kernel)),
            (NodeKind::KernelPull, (kernel::CMDLINE, Artifact::Cmdline)),
            (
                NodeKind::Concat,
                (initramfs::concat::CONCAT_OUTPUT, Artifact::Initramfs),
            ),
            (NodeKind::Uki, (uki::UKI_OUTPUT, Artifact::Uki)),
            (NodeKind::Iso, (media::MEDIA_OUTPUT, Artifact::Iso)),
            (NodeKind::Raw, (media::MEDIA_OUTPUT, Artifact::Raw)),
            (
                NodeKind::OverlayTar,
                (overlay::tar::TAR_OUTPUT, Artifact::Overlays),
            ),
        ];

        for (kind, (port, artifact)) in routes {
            // ACT
            let declared = produces(kind, &ctx);

            // ASSERT
            assert!(
                declared.contains(&(port, artifact)),
                "{kind:?} must declare {artifact} on port {port:?}"
            );
        }
    }

    #[test]
    fn unsigned_planning_produces_the_uki_from_the_uki_node_only() {
        // ARRANGE
        let build = build_plan();
        let ctx = context(&build);

        // ACT
        let sources: Vec<_> = NodeKind::ALL
            .iter()
            .flat_map(|kind| produces(*kind, &ctx))
            .filter(|&(_, artifact)| artifact == Artifact::Uki)
            .collect();

        // ASSERT
        assert_eq!(
            sources,
            vec![(uki::UKI_OUTPUT, Artifact::Uki)],
            "unsigned planning must route Uki through the Uki node only"
        );
    }

    #[test]
    fn signed_planning_produces_the_uki_from_the_sign_node_only() {
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
        let sources: Vec<_> = NodeKind::ALL
            .iter()
            .flat_map(|kind| produces(*kind, &ctx))
            .filter(|&(_, artifact)| artifact == Artifact::Uki)
            .collect();

        // ASSERT
        assert_eq!(
            sources,
            vec![(sign::SIGN_OUTPUT, Artifact::Uki)],
            "signed planning must route Uki through the Sign node only"
        );
    }
}
