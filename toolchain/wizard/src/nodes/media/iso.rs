//! ISO media image builder.

use std::io::Read;

use miso::iso;

use crate::artifact::Artifact;
use crate::domain::overlay::Asset;
use crate::error::{Result, WizardError};
use crate::nodes::media::{self, MEDIA_OUTPUT};
use crate::nodes::{NodeDescriptor, NodeKind};
use crate::pipeline::context::BuildContext;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::Graph;
use crate::pipeline::node::{NodeId, PortId};
use crate::pipeline::runtime::NodePorts;

pub(crate) const DESCRIPTOR: NodeDescriptor = NodeDescriptor {
    dependencies: media::dependencies,
    produces,
    preflight,
    run,
};

/// The bootable ISO image.
fn produces(_kind: NodeKind, _ctx: &BuildContext<'_, '_>) -> Vec<(PortId, Artifact)> {
    vec![(MEDIA_OUTPUT, Artifact::Iso)]
}

/// Exact ISO size from the UKI stream and the overlay ESP layout.
fn preflight(graph: &mut Graph, id: NodeId, ctx: &BuildContext<'_, '_>) -> Result<()> {
    let assets = ctx.build.overlay_assets().unwrap_or(&[]);
    let layout = media::layout(ctx, media::uki_size(graph, id)?, assets)?;
    let planned = iso::plan(&layout)
        .map_err(|e| WizardError::BuildError(format!("plan bootable ISO: {e}")))?;

    media::set_output(graph, id, planned.size(), "boot.iso")
}

/// Builds the ISO from the UKI stream and overlay ESP file streams.
fn run(
    _kind: NodeKind,
    ports: &mut NodePorts<'_, '_>,
    ctx: &BuildContext<'_, '_>,
) -> Result<NodeReport> {
    let (mut uki, mut overlays) = media::inputs(ports)?;
    let assets = ctx.build.overlay_assets().unwrap_or(&[]);
    if assets
        .iter()
        .any(|asset| matches!(asset, Asset::RawBlob { .. }))
    {
        return Err(WizardError::BuildError(
            "raw blobs cannot be written to an ISO image".to_owned(),
        ));
    }
    let layout = media::layout(ctx, uki.size, assets)?;
    let planned = iso::plan(&layout)
        .map_err(|e| WizardError::BuildError(format!("plan bootable ISO: {e}")))?;
    let mut output = ports.output(MEDIA_OUTPUT)?;
    media::check_size("ISO", planned.size(), output.size)?;

    let mut readers: Vec<&mut dyn Read> = Vec::with_capacity(overlays.len().saturating_add(1));
    readers.push(&mut uki.reader);
    for overlay in &mut overlays {
        readers.push(&mut overlay.reader);
    }

    iso::write(&planned, &layout, &mut readers, &mut output.writer)
        .map_err(|e| WizardError::BuildError(format!("build bootable ISO: {e}")))?;

    Ok(None)
}
