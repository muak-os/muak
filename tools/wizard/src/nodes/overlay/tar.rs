//! Creates a tar archive of overlay ESP files, with stripped paths and preflight sizes.

use tar::{Builder, Header};

use crate::error::{Result, WizardError};
use crate::nodes::overlay::discovery::{OverlayAsset, assets};
use crate::nodes::overlay::pull;
use crate::nodes::{NodeDescriptor, NodeKind, no_dynamic_output_count};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::{Graph, NodeId, PortId};
use crate::pipeline::runtime::{Endpoint, NodePorts};

pub(crate) const TAR_OUTPUT: PortId = PortId(0);
pub(crate) const TAR_INPUTS_FIRST: PortId = PortId(1);

pub(crate) const DESCRIPTOR: NodeDescriptor = NodeDescriptor {
    dependencies,
    output_count: no_dynamic_output_count,
    preflight,
    run,
};

/// One stream per overlay asset, in canonical (path-sorted) order.
fn dependencies(_kind: NodeKind, _ctx: &BuildContext<'_, '_, '_>) -> Vec<Dependency> {
    vec![Dependency::many(
        NodeKind::OverlayPull,
        pull::PULL_OUTPUTS_FIRST,
        TAR_INPUTS_FIRST,
    )]
}

/// tar size = headers + file data + padding per entry, plus the two zero trailer blocks.
fn preflight(graph: &mut Graph, id: NodeId, ctx: &BuildContext<'_, '_, '_>) -> Result<()> {
    let overlay = ctx
        .build
        .overlay()
        .ok_or_else(|| WizardError::BuildError("overlay tar has no overlay source".to_owned()))?;
    let esp_sizes: Vec<u64> = assets(overlay)?
        .iter()
        .filter_map(|asset| match *asset {
            OverlayAsset::EspFile { size, .. } => Some(size),
            OverlayAsset::RawBlob { .. } => None,
        })
        .collect();
    let tar = tar_total_size(&esp_sizes);
    let output = graph.stream_mut(graph.node(id)?.output(TAR_OUTPUT)?)?;
    output.size = tar;
    "overlays.tar".clone_into(&mut output.name);

    Ok(())
}

/// Emits one tar entry per overlay ESP file, skipping raw blobs.
fn run(
    _kind: NodeKind,
    ports: &mut NodePorts<'_>,
    ctx: &BuildContext<'_, '_, '_>,
) -> Result<NodeReport> {
    let overlay = ctx
        .build
        .overlay()
        .ok_or_else(|| WizardError::BuildError("overlay tar has no overlay source".to_owned()))?;
    let assets = assets(overlay)?;

    let mut inputs = Endpoint::into_inputs(
        ports
            .take_from(TAR_INPUTS_FIRST, None)?
            .into_iter()
            .map(|(_, endpoint)| endpoint),
    )?;
    let mut output = ports.take(TAR_OUTPUT)?.into_output()?;

    let mut builder = Builder::new(&mut output.writer);
    for (asset, input) in assets.iter().zip(inputs.iter_mut()) {
        let OverlayAsset::EspFile { ref path, .. } = *asset else {
            continue;
        };
        let mut header = Header::new_gnu();
        header
            .set_path(path)
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
