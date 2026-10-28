//! ISO and raw disk media image builders.

pub(crate) mod iso;
pub(crate) mod raw;

use esp::FileMeta;
use esp::layout::compute;

use crate::domain::overlay::Asset;
use crate::error::{Result, WizardError};
use crate::nodes::{NodeKind, overlay, sign, uki};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::graph::Graph;
use crate::pipeline::node::{NodeId, PortId};
use crate::pipeline::runtime::{InputStream, NodePorts};

pub(crate) const MEDIA_UKI: PortId = PortId(0);
pub(crate) const MEDIA_OUTPUT: PortId = PortId(1);
pub(crate) const MEDIA_OVERLAYS_FIRST: PortId = PortId(2);

/// The UKI stream, plus one stream per overlay asset when the build has overlay assets.
pub(crate) fn dependencies(_kind: NodeKind, ctx: &BuildContext<'_, '_>) -> Vec<Dependency> {
    let uki = if ctx.signing.is_some() {
        Dependency::new(NodeKind::Sign, sign::SIGN_OUTPUT, MEDIA_UKI)
    } else {
        Dependency::new(NodeKind::Uki, uki::UKI_OUTPUT, MEDIA_UKI)
    };
    let mut dependencies = vec![uki];
    if let Some(assets) = ctx.build.overlay_assets() {
        for index in 0..assets.len() {
            dependencies.push(Dependency::new(
                NodeKind::OverlayPull,
                overlay::pull::PULL_OUTPUTS_FIRST.offset(index),
                MEDIA_OVERLAYS_FIRST.offset(index),
            ));
        }
    }

    dependencies
}

/// The UKI stream plus one stream per overlay asset.
pub(crate) fn inputs<'name>(
    ports: &mut NodePorts<'name, '_>,
) -> Result<(InputStream<'name>, Vec<InputStream<'name>>)> {
    let uki = ports.input(MEDIA_UKI)?;
    let overlays = ports.inputs_from(MEDIA_OVERLAYS_FIRST, None)?;

    Ok((uki, overlays))
}

/// ESP layout for the UKI plus the overlay ESP files.
pub(crate) fn layout<'a>(
    ctx: &BuildContext<'_, '_>,
    uki_size: u64,
    assets: &'a [Asset],
) -> Result<esp::layout::Layout<'a>> {
    let mut file_metas = Vec::with_capacity(assets.len().saturating_add(1));
    file_metas.push(FileMeta::new(
        crate::arch::esp(ctx.build.arch()).boot_path(),
        uki_size,
    ));
    for asset in assets {
        if let Asset::EspFile { ref path, size, .. } = *asset {
            file_metas.push(FileMeta::new(path, size));
        }
    }

    compute(&file_metas).map_err(|e| WizardError::BuildError(format!("compute ESP layout: {e}")))
}

/// Size of the media node's UKI input stream.
pub(crate) fn uki_size(graph: &Graph, id: NodeId) -> Result<u64> {
    let node = graph.node(id)?;

    Ok(graph.stream(node.input(MEDIA_UKI)?)?.size)
}

/// Sets the media output stream size and name.
pub(crate) fn set_output(graph: &mut Graph, id: NodeId, size: u64, name: &str) -> Result<()> {
    let output = graph.stream_mut(graph.node(id)?.output(MEDIA_OUTPUT)?)?;
    output.size = size;
    name.clone_into(&mut output.name);

    Ok(())
}

/// Rejects a runtime plan size that differs from the preflight size.
pub(crate) fn check_size(kind: &str, planned: u64, output: u64) -> Result<()> {
    if planned != output {
        return Err(WizardError::BuildError(format!(
            "{kind} size mismatch: runtime {planned} != preflight {output}"
        )));
    }

    Ok(())
}
