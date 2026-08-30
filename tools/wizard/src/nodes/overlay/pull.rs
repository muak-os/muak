//! Single overlay pull routed to media and tar consumers.

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
    let mut files: Vec<(&'a str, &mut OutputStream<'a>)> = outputs
        .iter_mut()
        .map(|output| (output.name, output))
        .collect();
    let source = &overlay.source;
    let arch = &overlay.arch;

    let mut error: Option<WizardError> = None;
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for (name, output) in &mut files {
            let name = *name;
            let output = &mut *output;
            handles.push(scope.spawn(move || {
                pull::files(source, arch, None, |entry| {
                    if let Some(found) = entry_name(overlay, &entry.path) {
                        if found == name {
                            std::io::copy(entry.reader, &mut output.writer)
                                .map_err(KociError::IoError)?;
                        }
                    }
                    Ok(())
                })
                .map_err(|e| WizardError::BuildError(format!("pull overlay files: {e}")))
            }));
        }
        for handle in handles {
            match handle.join() {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    if error.is_none() {
                        error = Some(e);
                    }
                }
                Err(e) => {
                    if error.is_none() {
                        error = Some(WizardError::BuildError(format!(
                            "overlay pull panicked: {e:?}"
                        )));
                    }
                }
            }
        }
    });

    if let Some(e) = error {
        return Err(e);
    }

    Ok(NodeReport::Empty)
}

fn output_count(ctx: &BuildContext<'_, '_, '_>) -> Result<usize> {
    let overlay = ctx
        .build
        .overlay()
        .ok_or_else(|| WizardError::BuildError("overlay node has no overlay source".to_owned()))?;

    Ok(assets(overlay)?.len())
}
