//! Routes the installer initramfs into the build stream.

use koci::error::KociError;
use koci::pull;

use crate::artifact::Artifact;
use crate::error::{Result, WizardError};
use crate::nodes::{NodeDescriptor, NodeKind, measured_size};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::Graph;
use crate::pipeline::node::{NodeId, PortId};
use crate::pipeline::runtime::NodePorts;

pub(crate) const INITRAMFS: PortId = PortId(0);
pub(crate) const INITRAMFS_PATH: &str = "initramfs.img";

pub(crate) const DESCRIPTOR: NodeDescriptor = NodeDescriptor {
    dependencies,
    produces,
    preflight,
    run,
};

/// Source node meaning no dependencies.
fn dependencies(_kind: NodeKind, _ctx: &BuildContext<'_, '_>) -> Vec<Dependency> {
    Vec::new()
}

/// The installer produces no requested artifact directly.
fn produces(_kind: NodeKind, _ctx: &BuildContext<'_, '_>) -> Vec<(PortId, Artifact)> {
    Vec::new()
}

/// Exact initramfs size, measured from the image's eStargz TOC.
fn preflight(graph: &mut Graph, id: NodeId, ctx: &BuildContext<'_, '_>) -> Result<()> {
    let build = ctx.build;

    let size = measured_size(build.installer(), build.arch(), INITRAMFS_PATH)?;

    let binding = graph
        .node(id)?
        .output_bindings()
        .copied()
        .next()
        .ok_or_else(|| WizardError::BuildError("installer node has no output binding".into()))?;
    let stream = graph.stream_mut(binding.stream)?;
    stream.size = size;
    INITRAMFS_PATH.clone_into(&mut stream.name);

    Ok(())
}

/// Pulls the installer once and writes its initramfs to the output stream.
fn run(
    kind: NodeKind,
    ports: &mut NodePorts<'_, '_>,
    ctx: &BuildContext<'_, '_>,
) -> Result<NodeReport> {
    let mut output = ports.output(INITRAMFS)?;

    pull::files(
        ctx.build.installer(),
        &ctx.build.arch(),
        None,
        &ctx.reporter(kind),
        |mut entry| {
            if entry.path == INITRAMFS_PATH {
                std::io::copy(&mut entry.reader, &mut output.writer).map_err(KociError::IoError)?;
            }

            Ok(())
        },
    )
    .map_err(|e| WizardError::BuildError(format!("pull installer files: {e}")))?;

    Ok(None)
}
