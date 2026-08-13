//! Single overlay pull routed to media and tar consumers.

use std::io::Read;

use koci::error::KociError;
use koci::pull;
use tar::{Builder, Header};

use crate::error::{Result, WizardError};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::{Graph, NodeId, NodeKind, PortId};
use crate::pipeline::runtime::{Endpoint, NodePorts, OutputStream};
use crate::resolve::BuildPlan;
use crate::source::overlay::Overlay;

pub(crate) const PULL_OUTPUTS_FIRST: PortId = PortId(0);
pub(crate) const TAR_OUTPUT: PortId = PortId(0);
pub(crate) const TAR_INPUTS_FIRST: PortId = PortId(1);

/// Source node meaning no dependencies.
pub(crate) fn pull_dependencies() -> Vec<Dependency> {
    Vec::new()
}

/// One stream per overlay file, in canonical (path-sorted) order.
pub(crate) fn tar_dependencies() -> Vec<Dependency> {
    vec![Dependency::many(
        NodeKind::OverlayPull,
        PULL_OUTPUTS_FIRST,
        TAR_INPUTS_FIRST,
    )]
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

/// Sizes the overlay output streams from the listing and returns it for the media and tar runners.
pub(crate) async fn preflight_pull(
    graph: &mut Graph,
    id: NodeId,
    context: &BuildContext<'_, '_, '_>,
) -> Result<Vec<(String, u64)>> {
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
        graph.stream_mut(binding.stream)?.size = file.1;
    }

    Ok(files)
}

/// tar size = headers + file data + padding per entry, plus the two zero trailer blocks.
pub(crate) fn preflight_tar(graph: &mut Graph, id: NodeId) -> Result<()> {
    let files = graph
        .node(id)?
        .input_bindings()
        .map(|binding| Ok(graph.stream(binding.stream)?.size))
        .collect::<Result<Vec<_>>>()?;
    let tar = tar_total_size(&files);
    graph.stream_mut(graph.node(id)?.output(TAR_OUTPUT)?)?.size = tar;

    Ok(())
}

/// Pulls the overlay source once and streams each matching entry to the output stream.
pub(crate) fn run_pull(
    ctx: &BuildContext<'_, '_, '_>,
    overlay_files: &[(String, u64)],
    ports: &mut NodePorts,
    tokio: &tokio::runtime::Handle,
) -> Result<NodeReport> {
    let overlay = ctx
        .plan
        .overlay()
        .ok_or_else(|| WizardError::BuildError("overlay node has no overlay source".to_owned()))?;
    let outputs = Endpoint::into_outputs(
        ports
            .take_from(PULL_OUTPUTS_FIRST, Some(overlay_files.len()))?
            .into_iter()
            .map(|(_, endpoint)| endpoint),
    )?;
    let mut files: Vec<(&str, OutputStream)> = overlay_files
        .iter()
        .map(|file| file.0.as_str())
        .zip(outputs)
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

/// Emits one tar entry per overlay input with the stripped path and preflight size.
pub(crate) fn run_tar(
    overlay_files: &[(String, u64)],
    ports: &mut NodePorts,
) -> Result<NodeReport> {
    let mut inputs = Endpoint::into_inputs(
        ports
            .take_from(TAR_INPUTS_FIRST, Some(overlay_files.len()))?
            .into_iter()
            .map(|(_, endpoint)| endpoint),
    )?;
    let mut output = ports.take(TAR_OUTPUT)?.into_output()?;

    let mut builder = Builder::new(&mut output.writer);
    for (input, file) in inputs.iter_mut().zip(overlay_files) {
        let mut header = Header::new_gnu();
        header
            .set_path(&file.0)
            .map_err(|e| WizardError::BuildError(format!("set tar header path: {e}")))?;
        header.set_size(input.size);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append(&header, &mut input.reader)
            .map_err(|e| WizardError::BuildError(format!("append to tar: {e}")))?;
    }
    builder
        .finish()
        .map_err(|e| WizardError::BuildError(format!("finish overlay tar: {e}")))?;

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

fn route_entry(
    path: &str,
    reader: &mut dyn Read,
    prefix: &str,
    files: &mut [(&str, OutputStream)],
) -> koci::error::Result<()> {
    if let Some(rel) = path.strip_prefix(prefix)
        && !rel.is_empty()
    {
        write_to_matching(rel, reader, files).map_err(KociError::IoError)?;
    }

    Ok(())
}

fn write_to_matching(
    rel: &str,
    reader: &mut dyn Read,
    files: &mut [(&str, OutputStream)],
) -> std::io::Result<()> {
    let Some(index) = files.iter().position(|file| file.0 == rel) else {
        return Ok(());
    };
    if let Some(output) = files.get_mut(index).map(|item| &mut item.1) {
        std::io::copy(reader, &mut output.writer)?;
    }

    Ok(())
}

fn tar_total_size(files: &[u64]) -> u64 {
    let mut total = 1024_u64;
    for size in files {
        total = total
            .saturating_add(512)
            .saturating_add(size.div_ceil(512).saturating_mul(512));
    }

    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tar_total_size_matches_builder_output() {
        // ARRANGE
        let files: [u64; 2] = [5, 300];
        let expected = tar_total_size(&files);

        // ACT
        let mut buf = Vec::new();
        let mut builder = Builder::new(&mut buf);
        for (index, size) in files.iter().enumerate() {
            let mut header = Header::new_gnu();
            header
                .set_path(format!("file-{index}.txt"))
                .expect("set path");
            header.set_size(*size);
            header.set_mode(0o644);
            let mut data: &[u8] = &vec![0_u8; usize::try_from(*size).unwrap_or(0)];
            builder.append(&header, &mut data).expect("append");
        }
        builder.finish().expect("finish");
        drop(builder);

        // ASSERT
        assert_eq!(u64::try_from(buf.len()).unwrap_or(0), expected);
    }
}
