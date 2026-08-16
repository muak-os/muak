//! Build pipeline node runners.

pub(crate) mod extensions;
pub(crate) mod fanout;
pub(crate) mod initramfs;
pub(crate) mod installer;
pub(crate) mod media;
pub(crate) mod overlay;
pub(crate) mod sign;
pub(crate) mod sink;
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
    pub(crate) dependencies: fn(&BuildContext<'_, '_, '_>) -> Vec<Dependency>,
    pub(crate) output_count: fn(&BuildContext<'_, '_, '_>) -> Result<usize>,
    pub(crate) preflight: fn(&mut Graph, NodeId, &BuildContext<'_, '_, '_>) -> Result<()>,
    pub(crate) run: fn(&mut NodePorts<'_>, &BuildContext<'_, '_, '_>) -> Result<NodeReport>,
}

/// The kind → descriptor catalog.
pub(crate) fn descriptor(kind: NodeKind) -> Result<&'static NodeDescriptor> {
    match kind {
        NodeKind::InstallerPull => Ok(&installer::DESCRIPTOR),
        NodeKind::ExtensionPayloads => Ok(&extensions::DESCRIPTOR),
        NodeKind::InitramfsTail => Ok(&initramfs::tail::DESCRIPTOR),
        NodeKind::Concat => Ok(&initramfs::concat::DESCRIPTOR),
        NodeKind::Uki => Ok(&uki::DESCRIPTOR),
        NodeKind::Sign => Ok(&sign::DESCRIPTOR),
        NodeKind::Iso => Ok(&media::iso::DESCRIPTOR),
        NodeKind::Raw => Ok(&media::raw::DESCRIPTOR),
        NodeKind::OverlayPull => Ok(&overlay::pull::DESCRIPTOR),
        NodeKind::OverlayTar => Ok(&overlay::tar::DESCRIPTOR),
        NodeKind::Fanout => Ok(&fanout::DESCRIPTOR),
        NodeKind::ArtifactSink { .. } => Err(WizardError::BuildError(
            "sink nodes carry instance data and are handled separately".to_owned(),
        )),
    }
}

/// Declared inputs of a planned kind; sinks carry their requested artifact.
///
/// # Errors
///
/// Returns an error when the kind has no descriptor (sink nodes are handled
/// by the caller through the artifact-carrying variant).
pub(crate) fn dependencies(
    kind: NodeKind,
    ctx: &BuildContext<'_, '_, '_>,
) -> Result<Vec<Dependency>> {
    if let NodeKind::ArtifactSink { artifact } = kind {
        Ok(sink::dependencies(artifact, ctx))
    } else {
        let node = descriptor(kind)?;

        Ok((node.dependencies)(ctx))
    }
}

/// Dynamic output count of a producer kind, fetched once per plan.
pub(crate) fn output_count(kind: NodeKind, ctx: &BuildContext<'_, '_, '_>) -> Result<usize> {
    let node = descriptor(kind)?;

    (node.output_count)(ctx)
}

/// The shared `output_count` for kinds without a dynamic output range.
pub(crate) fn no_dynamic_output_count(_ctx: &BuildContext<'_, '_, '_>) -> Result<usize> {
    Err(WizardError::BuildError(
        "node kind has no dynamic output count".to_owned(),
    ))
}
