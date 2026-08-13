//! ISO and raw disk media image builders.

use std::io::Read;

use esp::FileMeta;
use esp::layout::compute;
use miso::{iso, raw};

use crate::error::{Result, WizardError};
use crate::nodes::{overlays, uki};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::{Graph, NodeId, NodeKind, PortId};
use crate::pipeline::runtime::{DynWriter, Endpoint, InputStream, NodePorts};
use crate::resolve::BuildPlan;

pub(crate) const MEDIA_UKI: PortId = PortId(0);
pub(crate) const MEDIA_OUTPUT: PortId = PortId(1);
pub(crate) const MEDIA_OVERLAYS_FIRST: PortId = PortId(2);

/// The UKI stream, plus overlay file streams when the profile has an overlay.
pub(crate) fn dependencies(build: &BuildPlan) -> Vec<Dependency> {
    let mut dependencies = vec![Dependency::fixed(NodeKind::Uki, uki::UKI_OUTPUT, MEDIA_UKI)];
    if build.overlay().is_some() {
        dependencies.push(Dependency::many(
            NodeKind::OverlayPull,
            overlays::PULL_OUTPUTS_FIRST,
            MEDIA_OVERLAYS_FIRST,
        ));
    }

    dependencies
}

/// The media output stream size is not needed so we set it to zero.
pub(crate) fn preflight(graph: &mut Graph, id: NodeId) -> Result<()> {
    graph
        .stream_mut(graph.node(id)?.output(MEDIA_OUTPUT)?)?
        .size = 0;

    Ok(())
}

/// Builds the ISO from the UKI stream and overlay file streams.
pub(crate) fn run_iso(
    ctx: &BuildContext<'_, '_, '_>,
    overlay_files: &[(String, u64)],
    ports: &mut NodePorts,
) -> Result<NodeReport> {
    let (layout, mut uki, mut overlays) = media_inputs(ctx, overlay_files, ports)?;
    let mut output = ports.take(MEDIA_OUTPUT)?.into_output()?;

    let mut readers: Vec<&mut dyn Read> = Vec::with_capacity(overlays.len().saturating_add(1));
    readers.push(&mut uki.reader);
    for overlay in &mut overlays {
        readers.push(&mut overlay.reader);
    }

    iso::build(
        &layout,
        &mut readers,
        &mut DynWriter::new(&mut output.writer),
    )
    .map_err(|e| WizardError::BuildError(format!("build bootable ISO: {e}")))?;

    Ok(NodeReport::Empty)
}

/// Builds the zstd-compressed raw disk image from the UKI stream and overlay file streams.
pub(crate) fn run_raw(
    ctx: &BuildContext<'_, '_, '_>,
    overlay_files: &[(String, u64)],
    ports: &mut NodePorts,
) -> Result<NodeReport> {
    let (layout, mut uki, mut overlays) = media_inputs(ctx, overlay_files, ports)?;
    let mut output = ports.take(MEDIA_OUTPUT)?.into_output()?;

    let mut readers: Vec<&mut dyn Read> = Vec::with_capacity(overlays.len().saturating_add(1));
    readers.push(&mut uki.reader);
    for overlay in &mut overlays {
        readers.push(&mut overlay.reader);
    }

    raw::build(
        &layout,
        &mut readers,
        &mut DynWriter::new(&mut output.writer),
        Some(6),
    )
    .map_err(|e| WizardError::BuildError(format!("build raw disk image: {e}")))?;

    Ok(NodeReport::Empty)
}

/// Takes the UKI and overlay inputs and computes the ESP layout.
fn media_inputs<'f>(
    ctx: &BuildContext<'_, '_, '_>,
    overlay_files: &'f [(String, u64)],
    ports: &mut NodePorts,
) -> Result<(esp::layout::Layout<'f>, InputStream, Vec<InputStream>)> {
    let uki = ports.take(MEDIA_UKI)?.into_input()?;
    let overlays = Endpoint::into_inputs(
        ports
            .take_from(MEDIA_OVERLAYS_FIRST, Some(overlay_files.len()))?
            .into_iter()
            .map(|(_, endpoint)| endpoint),
    )?;

    let mut file_metas = Vec::with_capacity(overlays.len().saturating_add(1));
    file_metas.push(FileMeta::new(
        crate::arch::esp(ctx.plan.arch()).boot_path(),
        uki.size,
    ));
    file_metas.extend(
        overlays
            .iter()
            .zip(overlay_files)
            .map(|(input, file)| FileMeta::new(file.0.as_str(), input.size)),
    );

    let layout = compute(&file_metas)
        .map_err(|e| WizardError::BuildError(format!("compute ESP layout: {e}")))?;

    Ok((layout, uki, overlays))
}
