//! Single overlay pull routed to media and tar consumers.

use std::io::Read;

use koci::error::KociError;
use koci::pull;

use crate::error::{Result, WizardError};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::{Graph, NodeId, PortId};
use crate::pipeline::runtime::{Endpoint, NodePorts, OutputStream};
use crate::resolve::BuildPlan;
use crate::source::overlay::Overlay;

pub(crate) const PULL_OUTPUTS_FIRST: PortId = PortId(0);

/// Source node meaning no dependencies.
pub(crate) fn dependencies() -> Vec<Dependency> {
    Vec::new()
}

/// Stripped overlay file paths plus sizes, path-sorted, with the same
/// `{name}/` prefix stripping the runtime pull applies.
pub(crate) async fn listing(overlay: &Overlay) -> Result<Vec<(String, u64)>> {
    let prefix = format!("{}/", overlay.name);
    let mut files: Vec<(String, u64)> = Vec::new();
    pull::metadata(&overlay.source, &overlay.arch, None, |entry| {
        if let Some(rel) = entry.path.strip_prefix(&prefix)
            && !rel.is_empty()
        {
            files.push((rel.to_owned(), entry.size));
        }
        Ok(())
    })
    .await
    .map_err(|e| WizardError::BuildError(format!("list overlay files: {e}")))?;
    files.sort_unstable_by(|left, right| left.0.cmp(&right.0));

    Ok(files)
}

/// Sizes and names the overlay output streams from the listing.
pub(crate) async fn preflight(
    graph: &mut Graph,
    id: NodeId,
    context: &BuildContext<'_, '_, '_>,
) -> Result<()> {
    let overlay = context
        .plan
        .overlay()
        .ok_or_else(|| WizardError::BuildError("overlay node has no overlay source".to_owned()))?;
    let files = listing(overlay).await?;

    let bindings = graph
        .node(id)?
        .output_bindings()
        .copied()
        .collect::<Vec<_>>();
    if bindings.len() != files.len() {
        return Err(WizardError::BuildError(format!(
            "overlay output/file count mismatch: {} != {}",
            bindings.len(),
            files.len(),
        )));
    }
    for (binding, file) in bindings.iter().zip(&files) {
        let stream = graph.stream_mut(binding.stream)?;
        stream.size = file.1;
        stream.name.clone_from(&file.0);
    }

    Ok(())
}

/// Pulls the overlay source once and routes each matching entry to its named output stream.
pub(crate) fn run<'a>(
    ctx: &BuildContext<'_, '_, '_>,
    ports: &mut NodePorts<'a>,
    tokio: &tokio::runtime::Handle,
) -> Result<NodeReport> {
    let overlay = ctx
        .plan
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
    let prefix = format!("{}/", overlay.name);

    tokio
        .block_on(async move {
            pull::files(&overlay.source, &overlay.arch, None, |entry| {
                route_entry(&entry.path, entry.reader, &prefix, &mut files)
            })
            .await
        })
        .map_err(|e| WizardError::BuildError(format!("pull overlay files: {e}")))?;

    Ok(NodeReport::Empty)
}

/// Number of overlay files under the `{name}/` prefix.
///
/// # Errors
///
/// Returns an error when the overlay file listing cannot be fetched.
pub(crate) async fn output_count(build: &BuildPlan) -> Result<usize> {
    let overlay = build
        .overlay()
        .ok_or_else(|| WizardError::BuildError("overlay node has no overlay source".to_owned()))?;

    Ok(listing(overlay).await?.len())
}

fn route_entry<'a>(
    path: &str,
    reader: &mut dyn Read,
    prefix: &str,
    files: &mut [(&'a str, &mut OutputStream<'a>)],
) -> koci::error::Result<()> {
    if let Some(rel) = path.strip_prefix(prefix)
        && !rel.is_empty()
    {
        write_to_matching(rel, reader, files).map_err(KociError::IoError)?;
    }

    Ok(())
}

fn write_to_matching<'a>(
    rel: &str,
    reader: &mut dyn Read,
    files: &mut [(&'a str, &mut OutputStream<'a>)],
) -> std::io::Result<()> {
    let Some(index) = files.iter().position(|file| file.0 == rel) else {
        return Ok(());
    };
    let Some(output) = files.get_mut(index).map(|file| &mut *file.1) else {
        return Ok(());
    };
    std::io::copy(reader, &mut output.writer)?;

    Ok(())
}
