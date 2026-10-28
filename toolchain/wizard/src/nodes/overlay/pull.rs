//! Overlay pull.

use std::collections::HashMap;

use koci::progress::Reporter;
use koci::pull;
use koci::pull::demux::Demux;
use koci::pull::session::Session;

use crate::artifact::Artifact;
use crate::domain::overlay::Asset;
use crate::error::{Result, WizardError};
use crate::nodes::{NodeDescriptor, NodeKind};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::Graph;
use crate::pipeline::node::{NodeId, PortId};
use crate::pipeline::runtime::{NodePorts, OutputStream};

pub(crate) const PULL_OUTPUTS_FIRST: PortId = PortId(0);

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

/// Overlay assets are inputs of the tar and media nodes, never requested artifacts.
fn produces(_kind: NodeKind, _ctx: &BuildContext<'_, '_>) -> Vec<(PortId, Artifact)> {
    Vec::new()
}

/// Sizes and names the overlay output streams from the discovered assets.
fn preflight(graph: &mut Graph, id: NodeId, ctx: &BuildContext<'_, '_>) -> Result<()> {
    let assets = ctx
        .build
        .overlay_assets()
        .ok_or_else(|| WizardError::BuildError("overlay node has no overlay source".to_owned()))?;

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
    for (binding, asset) in bindings.iter().zip(assets) {
        let stream = graph.stream_mut(binding.stream)?;
        stream.size = asset.size();
        asset.name().clone_into(&mut stream.name);
    }

    Ok(())
}

/// Pulls the overlay source with one demux pass.
fn run(
    kind: NodeKind,
    ports: &mut NodePorts<'_, '_>,
    ctx: &BuildContext<'_, '_>,
) -> Result<NodeReport> {
    let overlay = ctx
        .build
        .overlay()
        .ok_or_else(|| WizardError::BuildError("overlay node has no overlay source".to_owned()))?;
    let assets = ctx
        .build
        .overlay_assets()
        .ok_or_else(|| WizardError::BuildError("overlay node has no overlay source".to_owned()))?;
    let outputs = ports.outputs_from(PULL_OUTPUTS_FIRST, None)?;
    let session = pull::session::open(&overlay.source, &overlay.arch, None)
        .map_err(|e| WizardError::BuildError(format!("open overlay session: {e}")))?;

    demux_pass(&session, assets, outputs, &ctx.reporter(kind))?;

    Ok(None)
}

/// Single-pass demux: routes every entry to its output stream in one walk.
fn demux_pass(
    session: &Session,
    assets: &[Asset],
    outputs: Vec<OutputStream<'_>>,
    progress: &dyn Reporter,
) -> Result<()> {
    let sizes = assets
        .iter()
        .map(|asset| (asset.entry().to_owned(), asset.size()));
    let writers = assets
        .iter()
        .zip(outputs)
        .map(|(asset, output)| (asset.entry().to_owned(), output))
        .collect::<HashMap<_, _>>();
    let mut demux = Demux::by_name(writers).with_sizes(sizes);

    session
        .walk(progress, |entry| demux.route(entry))
        .map_err(|e| WizardError::BuildError(format!("demux overlay files: {e}")))
}
