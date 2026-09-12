//! Raw disk media image builder.

use std::io::Read;

use miso::raw;

use crate::artifact::Artifact;
use crate::domain::overlay::Asset;
use crate::error::{Result, WizardError};
use crate::nodes::media::{self, MEDIA_OUTPUT, media_inputs, media_layout};
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

/// The raw disk image output stream, transport-encoded per the request codec.
fn produces(_kind: NodeKind, _ctx: &BuildContext<'_, '_>) -> Vec<(PortId, Artifact)> {
    vec![(MEDIA_OUTPUT, Artifact::Raw)]
}

/// The media output stream size is not needed so we set it to zero.
fn preflight(graph: &mut Graph, id: NodeId, ctx: &BuildContext<'_, '_>) -> Result<()> {
    let output = graph.stream_mut(graph.node(id)?.output(MEDIA_OUTPUT)?)?;
    output.size = 0;
    output.name = Artifact::Raw.output_name(ctx.codec);

    Ok(())
}

/// Builds the raw disk image from the UKI stream, overlay ESP files, and any
/// raw boot blobs written at their fixed offsets, transport-encoded with the
/// request codec.
fn run(
    _kind: NodeKind,
    ports: &mut NodePorts<'_, '_>,
    ctx: &BuildContext<'_, '_>,
) -> Result<NodeReport> {
    let (mut uki, mut overlays) = media_inputs(ports)?;
    let assets = ctx.build.overlay_assets().unwrap_or(&[]);
    let layout = media_layout(ctx, &uki, assets)?;

    let mut esp_readers: Vec<&mut dyn Read> = Vec::new();
    let mut raw_blobs: Vec<raw::Blob> = Vec::new();
    for (asset, input) in assets.iter().zip(overlays.iter_mut()) {
        match *asset {
            Asset::EspFile { .. } => esp_readers.push(&mut input.reader),
            Asset::RawBlob { offset, size, .. } => {
                raw_blobs.push(raw::Blob {
                    offset,
                    size,
                    reader: &mut input.reader,
                });
            }
        }
    }

    let mut readers_with_uki: Vec<&mut dyn Read> =
        Vec::with_capacity(esp_readers.len().saturating_add(1));
    readers_with_uki.push(&mut uki.reader);
    readers_with_uki.append(&mut esp_readers);

    let mut output = ports.output(MEDIA_OUTPUT)?;
    let mut encoder = ctx
        .codec
        .encoder(&mut output.writer)
        .map_err(|e| WizardError::BuildError(format!("wrap raw disk codec: {e}")))?;
    raw::build(&layout, &mut readers_with_uki, &mut raw_blobs, &mut encoder)
        .map_err(|e| WizardError::BuildError(format!("build raw disk image: {e}")))?;
    encoder
        .finish()
        .map_err(|e| WizardError::BuildError(format!("finish raw disk codec: {e}")))?;

    Ok(None)
}
