//! ISO media image builder.

use std::io::Read;

use miso::iso;

use crate::domain::overlay::Asset;
use crate::error::{Result, WizardError};
use crate::nodes::media::{self, MEDIA_OUTPUT, media_inputs, media_layout};
use crate::nodes::{NodeDescriptor, NodeKind};
use crate::pipeline::context::BuildContext;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::{Graph, NodeId};
use crate::pipeline::runtime::NodePorts;

pub(crate) const DESCRIPTOR: NodeDescriptor = NodeDescriptor {
    dependencies: media::dependencies,
    preflight,
    run,
};

/// The media output stream size is not needed so we set it to zero.
fn preflight(graph: &mut Graph, id: NodeId, _ctx: &BuildContext<'_, '_, '_>) -> Result<()> {
    let output = graph.stream_mut(graph.node(id)?.output(MEDIA_OUTPUT)?)?;
    output.size = 0;
    "boot.iso".clone_into(&mut output.name);

    Ok(())
}

/// Builds the ISO from the UKI stream and overlay ESP file streams.
fn run(
    _kind: NodeKind,
    ports: &mut NodePorts<'_>,
    ctx: &BuildContext<'_, '_, '_>,
) -> Result<NodeReport> {
    let (mut uki, mut overlays) = media_inputs(ports)?;
    let assets = ctx.build.overlay_assets().unwrap_or(&[]);
    if assets
        .iter()
        .any(|asset| matches!(asset, Asset::RawBlob { .. }))
    {
        return Err(WizardError::BuildError(
            "raw blobs cannot be written to an ISO image".to_owned(),
        ));
    }
    let layout = media_layout(ctx, &uki, assets)?;
    let mut output = ports.take(MEDIA_OUTPUT)?.into_output()?;

    let mut readers: Vec<&mut dyn Read> = Vec::with_capacity(overlays.len().saturating_add(1));
    readers.push(&mut uki.reader);
    for overlay in &mut overlays {
        readers.push(&mut overlay.reader);
    }

    iso::build(&layout, &mut readers, &mut output.writer)
        .map_err(|e| WizardError::BuildError(format!("build bootable ISO: {e}")))?;

    Ok(NodeReport::Empty)
}
