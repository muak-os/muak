//! Single overlay pull routed to media and tar consumers.

use std::thread::ScopedJoinHandle;

use koci::arch::Arch;
use koci::error::KociError;
use koci::pull;

use crate::domain::overlay::entry_name;
use crate::domain::resolution::Overlay;
use crate::error::{Result, WizardError};
use crate::nodes::{NodeDescriptor, NodeKind};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::{Graph, NodeId, PortId};
use crate::pipeline::runtime::{Endpoint, NodePorts, OutputStream};

pub(crate) const PULL_OUTPUTS_FIRST: PortId = PortId(0);

pub(crate) const DESCRIPTOR: NodeDescriptor = NodeDescriptor {
    dependencies,
    preflight,
    run,
};

/// Source node meaning no dependencies.
fn dependencies(_kind: NodeKind, _ctx: &BuildContext<'_, '_, '_>) -> Vec<Dependency> {
    Vec::new()
}

/// Sizes and names the overlay output streams from the discovered assets.
fn preflight(graph: &mut Graph, id: NodeId, ctx: &BuildContext<'_, '_, '_>) -> Result<()> {
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

/// Pulls the overlay source once per output stream, each on its own thread.
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
    let files: Vec<(&'a str, &mut OutputStream<'a>)> = outputs
        .iter_mut()
        .map(|output| (output.name, output))
        .collect();
    let source = &overlay.source;
    let arch = overlay.arch;

    let mut error: Option<WizardError> = None;
    std::thread::scope(|scope| {
        let handles = files
            .into_iter()
            .map(|(name, output)| {
                scope.spawn(move || pull_into_output(source, arch, overlay, name, output))
            })
            .collect::<Vec<_>>();
        error = collect_first_error(handles);
    });

    error.map_or(Ok(NodeReport::Empty), Err)
}

fn pull_into_output(
    source: &str,
    arch: Arch,
    overlay: &Overlay,
    name: &str,
    output: &mut OutputStream<'_>,
) -> Result<()> {
    pull::files(source, &arch, None, |entry| {
        if let Some(found) = entry_name(overlay, &entry.path)
            && found == name
        {
            std::io::copy(entry.reader, &mut output.writer).map_err(KociError::IoError)?;
        }

        Ok(())
    })
    .map_err(|e| WizardError::BuildError(format!("pull overlay files: {e}")))
}

fn collect_first_error(handles: Vec<ScopedJoinHandle<'_, Result<()>>>) -> Option<WizardError> {
    let mut error = None;
    for handle in handles {
        match handle.join() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                error.get_or_insert(e);
            }
            Err(e) => {
                error.get_or_insert(WizardError::BuildError(format!(
                    "overlay pull panicked: {e:?}"
                )));
            }
        }
    }

    error
}
