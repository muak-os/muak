//! ISO media image builder.

use std::io::Read;

use miso::iso;

use crate::error::{Result, WizardError};
use crate::nodes::media::{MEDIA_OUTPUT, media_inputs, media_layout};
use crate::pipeline::context::BuildContext;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::{Graph, NodeId};
use crate::pipeline::runtime::NodePorts;

/// The media output stream size is not needed so we set it to zero.
pub(crate) fn preflight(graph: &mut Graph, id: NodeId) -> Result<()> {
    let output = graph.stream_mut(graph.node(id)?.output(MEDIA_OUTPUT)?)?;
    output.size = 0;
    "boot.iso".clone_into(&mut output.name);

    Ok(())
}

/// Builds the ISO from the UKI stream and overlay file streams.
pub(crate) fn run(ctx: &BuildContext<'_, '_, '_>, ports: &mut NodePorts<'_>) -> Result<NodeReport> {
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
