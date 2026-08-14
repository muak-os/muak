//! Creates a tar archive of overlay files, with stripped paths and preflight sizes.

use tar::{Builder, Header};

use crate::error::{Result, WizardError};
use crate::nodes::overlay::pull;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::{Graph, NodeId, NodeKind, PortId};
use crate::pipeline::runtime::{Endpoint, NodePorts};

pub(crate) const TAR_OUTPUT: PortId = PortId(0);
pub(crate) const TAR_INPUTS_FIRST: PortId = PortId(1);

/// One stream per overlay file, in canonical (path-sorted) order.
pub(crate) fn dependencies() -> Vec<Dependency> {
    vec![Dependency::many(
        NodeKind::OverlayPull,
        pull::PULL_OUTPUTS_FIRST,
        TAR_INPUTS_FIRST,
    )]
}

/// tar size = headers + file data + padding per entry, plus the two zero trailer blocks.
pub(crate) fn preflight(graph: &mut Graph, id: NodeId) -> Result<()> {
    let files = graph
        .node(id)?
        .input_bindings()
        .map(|binding| Ok(graph.stream(binding.stream)?.size))
        .collect::<Result<Vec<_>>>()?;
    let tar = tar_total_size(&files);
    graph.stream_mut(graph.node(id)?.output(TAR_OUTPUT)?)?.size = tar;

    Ok(())
}

/// Emits one tar entry per overlay input with the stripped path and preflight size.
pub(crate) fn run(overlay_files: &[(String, u64)], ports: &mut NodePorts) -> Result<NodeReport> {
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
