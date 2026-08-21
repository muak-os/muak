//! Build pipeline node runners.

pub(crate) mod extensions;
pub(crate) mod fanout;
pub(crate) mod initramfs;
pub(crate) mod installer;
pub(crate) mod kernel;
pub(crate) mod media;
pub(crate) mod overlay;
pub(crate) mod sign;
pub(crate) mod sink;
pub(crate) mod stub;
pub(crate) mod uki;

use crate::artifact::Artifact;
use crate::error::{Result, WizardError};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::{Graph, NodeId};
use crate::pipeline::runtime::NodePorts;

/// What a node does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum NodeKind {
    InstallerPull,
    StubPull,
    KernelPull,
    ExtensionPayloads,
    InitramfsTail,
    Concat,
    Uki,
    Sign,
    Iso,
    Raw,
    OverlayPull,
    OverlayTar,
    ArtifactSink { artifact: Artifact },
    Fanout,
}

/// One node kind's full contract.
pub(crate) struct NodeDescriptor {
    pub(crate) dependencies: fn(NodeKind, &BuildContext<'_, '_, '_>) -> Vec<Dependency>,
    pub(crate) output_count: fn(&BuildContext<'_, '_, '_>) -> Result<usize>,
    pub(crate) preflight: fn(&mut Graph, NodeId, &BuildContext<'_, '_, '_>) -> Result<()>,
    pub(crate) run:
        fn(NodeKind, &mut NodePorts<'_>, &BuildContext<'_, '_, '_>) -> Result<NodeReport>,
}

/// The kind → descriptor catalog.
pub(crate) fn descriptor(kind: NodeKind) -> &'static NodeDescriptor {
    match kind {
        NodeKind::InstallerPull => &installer::DESCRIPTOR,
        NodeKind::StubPull => &stub::DESCRIPTOR,
        NodeKind::KernelPull => &kernel::DESCRIPTOR,
        NodeKind::ExtensionPayloads => &extensions::DESCRIPTOR,
        NodeKind::InitramfsTail => &initramfs::tail::DESCRIPTOR,
        NodeKind::Concat => &initramfs::concat::DESCRIPTOR,
        NodeKind::Uki => &uki::DESCRIPTOR,
        NodeKind::Sign => &sign::DESCRIPTOR,
        NodeKind::Iso => &media::iso::DESCRIPTOR,
        NodeKind::Raw => &media::raw::DESCRIPTOR,
        NodeKind::OverlayPull => &overlay::pull::DESCRIPTOR,
        NodeKind::OverlayTar => &overlay::tar::DESCRIPTOR,
        NodeKind::Fanout => &fanout::DESCRIPTOR,
        NodeKind::ArtifactSink { .. } => &sink::DESCRIPTOR,
    }
}

/// Declared inputs of a planned kind, read from its descriptor.
pub(crate) fn dependencies(kind: NodeKind, ctx: &BuildContext<'_, '_, '_>) -> Vec<Dependency> {
    (descriptor(kind).dependencies)(kind, ctx)
}

/// Dynamic output count of a producer kind, fetched once per plan.
pub(crate) fn output_count(kind: NodeKind, ctx: &BuildContext<'_, '_, '_>) -> Result<usize> {
    (descriptor(kind).output_count)(ctx)
}

/// The shared `output_count` for kinds without a dynamic output range.
pub(crate) fn no_dynamic_output_count(_ctx: &BuildContext<'_, '_, '_>) -> Result<usize> {
    Err(WizardError::BuildError(
        "node kind has no dynamic output count".to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use koci::arch::Arch;
    use sbolt::keys::SigningPair;
    use sbolt::keys::cert::generate_pk;

    use super::*;
    use crate::domain::resolution::Kernel;
    use crate::domain::resolution::{ResolvedBuild, Sources};
    use crate::pipeline::context::TargetWriters;
    use crate::request::Platform;

    fn build_plan() -> ResolvedBuild {
        ResolvedBuild::new(
            Platform::Metal,
            "v1.0.0".to_owned(),
            Arch::Amd64,
            Sources {
                stub: "ghcr.io/muak-os/pkgs/stub:v1.0.0".to_owned(),
                installer: "ghcr.io/muak-os/installer:v1.0.0".to_owned(),
                kernel: Kernel::new(
                    "ghcr.io/muak-os/kernel".to_owned(),
                    "ghcr.io/muak-os/kernel:v1.0.0".to_owned(),
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
            writers: std::sync::Mutex::new(TargetWriters::new(Vec::new())),
        }
    }

    #[test]
    fn sink_dependencies_route_artifacts_through_the_descriptor_table() {
        // ARRANGE
        let build = build_plan();
        let ctx = context(&build);
        let routes = [
            (Artifact::Kernel, NodeKind::KernelPull, kernel::KERNEL),
            (Artifact::Cmdline, NodeKind::KernelPull, kernel::CMDLINE),
            (
                Artifact::Initramfs,
                NodeKind::Concat,
                initramfs::concat::CONCAT_OUTPUT,
            ),
            (Artifact::Uki, NodeKind::Uki, uki::UKI_OUTPUT),
            (Artifact::Iso, NodeKind::Iso, media::MEDIA_OUTPUT),
            (Artifact::Raw, NodeKind::Raw, media::MEDIA_OUTPUT),
            (
                Artifact::Overlays,
                NodeKind::OverlayTar,
                overlay::tar::TAR_OUTPUT,
            ),
        ];

        for (artifact, producer, port) in routes {
            // ACT
            let declared = dependencies(NodeKind::ArtifactSink { artifact }, &ctx);

            // ASSERT
            assert_eq!(
                declared,
                vec![Dependency::fixed(producer, port, sink::SINK_INPUT)],
                "wrong sink dependency for {artifact}"
            );
        }
    }

    #[test]
    fn signed_uki_routes_the_sink_through_sign() {
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
            writers: std::sync::Mutex::new(TargetWriters::new(Vec::new())),
        };

        // ACT
        let declared = dependencies(
            NodeKind::ArtifactSink {
                artifact: Artifact::Uki,
            },
            &ctx,
        );

        // ASSERT
        assert_eq!(
            declared,
            vec![Dependency::fixed(
                NodeKind::Sign,
                sign::SIGN_OUTPUT,
                sink::SINK_INPUT
            )]
        );
    }
}
