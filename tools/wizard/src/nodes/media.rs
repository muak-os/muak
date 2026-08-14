//! ISO and raw disk media image builders.

use std::io::Read;

use esp::FileMeta;
use esp::layout::compute;
use miso::{iso, raw};

use crate::error::{Result, WizardError};
use crate::nodes::overlay;
use crate::nodes::{sign, uki};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::{Graph, NodeId, NodeKind, PortId};
use crate::pipeline::runtime::{Endpoint, InputStream, NodePorts};

pub(crate) const MEDIA_UKI: PortId = PortId(0);
pub(crate) const MEDIA_OUTPUT: PortId = PortId(1);
pub(crate) const MEDIA_OVERLAYS_FIRST: PortId = PortId(2);

/// The UKI stream, plus overlay file streams when the profile has an overlay.
pub(crate) fn dependencies(context: &BuildContext<'_, '_, '_>) -> Vec<Dependency> {
    let uki = if context.signing.is_some() {
        Dependency::fixed(NodeKind::Sign, sign::SIGN_OUTPUT, MEDIA_UKI)
    } else {
        Dependency::fixed(NodeKind::Uki, uki::UKI_OUTPUT, MEDIA_UKI)
    };
    let mut dependencies = vec![uki];
    if context.plan.overlay().is_some() {
        dependencies.push(Dependency::many(
            NodeKind::OverlayPull,
            overlay::pull::PULL_OUTPUTS_FIRST,
            MEDIA_OVERLAYS_FIRST,
        ));
    }

    dependencies
}

/// The media output stream size is not needed so we set it to zero.
pub(crate) fn preflight(graph: &mut Graph, id: NodeId) -> Result<()> {
    let kind = graph.node(id)?.kind;
    let name = if kind == NodeKind::Iso {
        "boot.iso"
    } else {
        "disk.raw"
    };
    let output = graph.stream_mut(graph.node(id)?.output(MEDIA_OUTPUT)?)?;
    output.size = 0;
    name.clone_into(&mut output.name);

    Ok(())
}

/// Builds the ISO from the UKI stream and overlay file streams.
pub(crate) fn run_iso(
    ctx: &BuildContext<'_, '_, '_>,
    ports: &mut NodePorts<'_>,
) -> Result<NodeReport> {
    let (mut uki, mut overlays) = media_inputs(ports)?;
    let layout = media_layout(ctx, &uki, &overlays)?;
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

/// Builds the zstd-compressed raw disk image from the UKI stream and overlay file streams.
pub(crate) fn run_raw(
    ctx: &BuildContext<'_, '_, '_>,
    ports: &mut NodePorts<'_>,
) -> Result<NodeReport> {
    let (mut uki, mut overlays) = media_inputs(ports)?;
    let layout = media_layout(ctx, &uki, &overlays)?;
    let mut output = ports.take(MEDIA_OUTPUT)?.into_output()?;

    let mut readers: Vec<&mut dyn Read> = Vec::with_capacity(overlays.len().saturating_add(1));
    readers.push(&mut uki.reader);
    for overlay in &mut overlays {
        readers.push(&mut overlay.reader);
    }

    raw::build(&layout, &mut readers, &mut output.writer, Some(6))
        .map_err(|e| WizardError::BuildError(format!("build raw disk image: {e}")))?;

    Ok(NodeReport::Empty)
}

fn media_inputs<'a>(ports: &mut NodePorts<'a>) -> Result<(InputStream<'a>, Vec<InputStream<'a>>)> {
    let uki = ports.take(MEDIA_UKI)?.into_input()?;
    let overlays = Endpoint::into_inputs(
        ports
            .take_from(MEDIA_OVERLAYS_FIRST, None)?
            .into_iter()
            .map(|(_, endpoint)| endpoint),
    )?;

    Ok((uki, overlays))
}

fn media_layout<'a>(
    ctx: &BuildContext<'_, '_, '_>,
    uki: &InputStream<'a>,
    overlays: &[InputStream<'a>],
) -> Result<esp::layout::Layout<'a>> {
    let mut file_metas = Vec::with_capacity(overlays.len().saturating_add(1));
    file_metas.push(FileMeta::new(
        crate::arch::esp(ctx.plan.arch()).boot_path(),
        uki.size,
    ));
    file_metas.extend(
        overlays
            .iter()
            .map(|input| FileMeta::new(input.name, input.size)),
    );

    compute(&file_metas).map_err(|e| WizardError::BuildError(format!("compute ESP layout: {e}")))
}
