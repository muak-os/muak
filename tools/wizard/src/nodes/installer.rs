//! Routes the installer initramfs into the build stream.

use std::collections::HashMap;

use koci::error::KociError;
use koci::pull;
use koci::pull::entries::MetadataEntry;

use crate::artifact::Artifact;
use crate::error::{Result, WizardError};
use crate::nodes::{NodeDescriptor, NodeKind};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::{Graph, NodeId, PortId};
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

/// Exact tar-entry size via the koci metadata callback.
fn preflight(graph: &mut Graph, id: NodeId, ctx: &BuildContext<'_, '_>) -> Result<()> {
    let build = ctx.build;

    let mut sizes = HashMap::new();
    pull::metadata(
        build.installer(),
        &build.arch(),
        None,
        |entry: MetadataEntry| {
            sizes.insert(entry.path, entry.size);

            Ok(())
        },
    )
    .map_err(|e| WizardError::BuildError(format!("extract installer metadata: {e}")))?;

    let size = sizes.get(INITRAMFS_PATH).copied().ok_or_else(|| {
        WizardError::BuildError(format!("missing installer size for {INITRAMFS_PATH}"))
    })?;

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
    _kind: NodeKind,
    ports: &mut NodePorts<'_, '_>,
    ctx: &BuildContext<'_, '_>,
) -> Result<NodeReport> {
    let mut output = ports
        .take_from(INITRAMFS, None)?
        .into_iter()
        .next()
        .ok_or_else(|| WizardError::BuildError("installer node has no output".into()))?
        .1
        .into_output()?;

    pull::files(
        ctx.build.installer(),
        &ctx.build.arch(),
        None,
        |mut entry| {
            if entry.path == INITRAMFS_PATH {
                std::io::copy(&mut entry.reader, &mut output.writer).map_err(KociError::IoError)?;
            }

            Ok(())
        },
    )
    .map_err(|e| WizardError::BuildError(format!("pull installer files: {e}")))?;

    Ok(NodeReport::Empty)
}
