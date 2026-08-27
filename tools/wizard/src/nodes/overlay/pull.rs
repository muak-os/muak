//! Single overlay pull routed to media and tar consumers.

use std::io::Read;

use koci::error::KociError;
use koci::pull;

use crate::error::{Result, WizardError};
use crate::nodes::overlay::discovery::{assets, entry_name};
use crate::nodes::{NodeDescriptor, NodeKind};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::{Graph, NodeId, PortId};
use crate::pipeline::runtime::{Endpoint, NodePorts, OutputStream};

pub(crate) const PULL_OUTPUTS_FIRST: PortId = PortId(0);

pub(crate) const DESCRIPTOR: NodeDescriptor = NodeDescriptor {
    dependencies,
    output_count,
    preflight,
    run,
};

/// Source node meaning no dependencies.
fn dependencies(_kind: NodeKind, _ctx: &BuildContext<'_, '_, '_>) -> Vec<Dependency> {
    Vec::new()
}

/// Sizes and names the overlay output streams from the asset listing.
fn preflight(graph: &mut Graph, id: NodeId, ctx: &BuildContext<'_, '_, '_>) -> Result<()> {
    let overlay = ctx
        .build
        .overlay()
        .ok_or_else(|| WizardError::BuildError("overlay node has no overlay source".to_owned()))?;
    let assets = assets(overlay)?;

    let bindings = graph
        .node(id)?
        .output_bindings()
        .copied()
        .collect::<Vec<_>>();
    if bindings.len() != assets.len() {
        return Err(WizardError::BuildError(format!(
            "overlay output/asset count mismatch: {} != {}",
            bindings.len(),
            assets.len(),
        )));
    }
    for (binding, asset) in bindings.iter().zip(&assets) {
        let stream = graph.stream_mut(binding.stream)?;
        stream.size = asset.size();
        asset.name().clone_into(&mut stream.name);
    }

    Ok(())
}

/// Pulls the overlay source once and routes each matching entry to its named output stream by its stripped asset path.
fn run<'a>(
    _kind: NodeKind,
    ports: &mut NodePorts<'a>,
    ctx: &BuildContext<'_, '_, '_>,
) -> Result<NodeReport> {
    let overlay = ctx
        .build
        .overlay()
        .ok_or_else(|| WizardError::BuildError("overlay node has no overlay source".to_owned()))?;
    let mut outputs = Endpoint::into_outputs(
        ports
            .take_from(PULL_OUTPUTS_FIRST, None)?
            .into_iter()
            .map(|(_, endpoint)| endpoint),
    )?;
    let mut files: Vec<(&'a str, &mut OutputStream<'a>)> = outputs
        .iter_mut()
        .map(|output| (output.name, output))
        .collect();

    pull::files(&overlay.source, &overlay.arch, None, |entry| {
        if let Some(name) = entry_name(overlay, &entry.path) {
            write_to_matching(&name, entry.reader, &mut files).map_err(KociError::IoError)?;
        }
        Ok(())
    })
    .map_err(|e| WizardError::BuildError(format!("pull overlay files: {e}")))?;

    Ok(NodeReport::Empty)
}

fn output_count(ctx: &BuildContext<'_, '_, '_>) -> Result<usize> {
    let overlay = ctx
        .build
        .overlay()
        .ok_or_else(|| WizardError::BuildError("overlay node has no overlay source".to_owned()))?;

    Ok(assets(overlay)?.len())
}

fn write_to_matching<'a>(
    name: &str,
    reader: &mut dyn Read,
    files: &mut [(&'a str, &mut OutputStream<'a>)],
) -> std::io::Result<()> {
    let Some(index) = files.iter().position(|file| file.0 == name) else {
        return Ok(());
    };
    let Some(output) = files.get_mut(index).map(|file| &mut *file.1) else {
        return Ok(());
    };
    std::io::copy(reader, &mut output.writer)?;

    Ok(())
}
