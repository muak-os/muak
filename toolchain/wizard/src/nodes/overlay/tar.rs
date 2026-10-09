//! Creates a tar archive of overlay ESP files, with stripped paths and preflight sizes.

use std::io::{self, Read as _};

use tar::{Builder, Header};

use crate::artifact::Artifact;
use crate::domain::overlay::Asset;
use crate::error::{Result, WizardError};
use crate::nodes::overlay::pull;
use crate::nodes::{NodeDescriptor, NodeKind};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::Graph;
use crate::pipeline::node::{NodeId, PortId};
use crate::pipeline::runtime::NodePorts;

pub(crate) const TAR_OUTPUT: PortId = PortId(0);
pub(crate) const TAR_INPUTS_FIRST: PortId = PortId(1);

pub(crate) const DESCRIPTOR: NodeDescriptor = NodeDescriptor {
    dependencies,
    produces,
    preflight,
    run,
};

/// One stream per overlay asset, in canonical (path-sorted) order.
fn dependencies(_kind: NodeKind, ctx: &BuildContext<'_, '_>) -> Vec<Dependency> {
    let assets = ctx.build.overlay_assets().unwrap_or(&[]);
    let mut dependencies = Vec::with_capacity(assets.len());
    for index in 0..assets.len() {
        dependencies.push(Dependency::new(
            NodeKind::OverlayPull,
            pull::PULL_OUTPUTS_FIRST.offset(index),
            TAR_INPUTS_FIRST.offset(index),
        ));
    }

    dependencies
}

/// The overlay assets tar archive.
fn produces(_kind: NodeKind, _ctx: &BuildContext<'_, '_>) -> Vec<(PortId, Artifact)> {
    vec![(TAR_OUTPUT, Artifact::Overlays)]
}

/// tar size = headers + file data + padding per entry, plus the two zero trailer blocks.
fn preflight(graph: &mut Graph, id: NodeId, ctx: &BuildContext<'_, '_>) -> Result<()> {
    let assets = ctx
        .build
        .overlay_assets()
        .ok_or_else(|| WizardError::BuildError("overlay tar has no overlay source".to_owned()))?;
    let esp_sizes: Vec<u64> = assets
        .iter()
        .filter_map(|asset| match *asset {
            Asset::EspFile { size, .. } => Some(size),
            Asset::RawBlob { .. } => None,
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
    ports: &mut NodePorts<'_, '_>,
    ctx: &BuildContext<'_, '_>,
) -> Result<NodeReport> {
    let assets = ctx
        .build
        .overlay_assets()
        .ok_or_else(|| WizardError::BuildError("overlay tar has no overlay source".to_owned()))?;

    let mut inputs = ports.inputs_from(TAR_INPUTS_FIRST, None)?;
    let mut output = ports.output(TAR_OUTPUT)?;

    let mut builder = Builder::new(&mut output.writer);
    for (asset, input) in assets.iter().zip(inputs.iter_mut()) {
        match *asset {
            Asset::EspFile { ref path, .. } => {
                let mut header = Header::new_gnu();
                header
                    .set_path(path)
                    .map_err(|e| WizardError::BuildError(format!("set tar header path: {e}")))?;
                header.set_size(input.size);
                header.set_mode(0o644);
                header.set_cksum();
                let mut member = input.reader.by_ref().take(input.size);
                builder
                    .append(&header, &mut member)
                    .map_err(|e| WizardError::BuildError(format!("append {path} to tar: {e}")))?;
                guard_full(member.limit(), path)?;
            }
            Asset::RawBlob { .. } => {
                let mut member = input.reader.by_ref().take(input.size);
                io::copy(&mut member, &mut io::sink())
                    .map_err(|e| WizardError::BuildError(format!("drain {}: {e}", input.name)))?;
                guard_full(member.limit(), input.name)?;
            }
        }
    }
    builder
        .finish()
        .map_err(|e| WizardError::BuildError(format!("finish overlay tar: {e}")))?;

    Ok(None)
}

fn guard_full(shortfall: u64, name: &str) -> Result<()> {
    if shortfall != 0 {
        return Err(WizardError::BuildError(format!(
            "asset {name}: stream ended {shortfall} bytes short of its size"
        )));
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
    use std::io::Read;

    use super::*;

    struct Endless<'a> {
        data: &'a [u8],
    }

    impl Read for Endless<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            buf.iter_mut()
                .enumerate()
                .for_each(|(index, byte)| *byte = self.data.get(index).copied().unwrap_or(0));

            Ok(1)
        }
    }

    fn entry(data: &[u8]) -> Vec<u8> {
        let len = u64::try_from(data.len()).expect("entry size overflows u64");
        let mut buf = Vec::new();
        let mut builder = Builder::new(&mut buf);
        let mut header = Header::new_gnu();
        header.set_path("asset.bin").expect("set path");
        header.set_size(len);
        header.set_mode(0o644);
        header.set_cksum();
        let mut member = Endless { data }.take(len);
        builder.append(&header, &mut member).expect("append");
        guard_full(member.limit(), "asset.bin").expect("guard");
        builder.finish().expect("finish");
        drop(builder);

        buf
    }

    #[test]
    fn copies_exact_size_from_a_source_that_never_signals_eof() {
        // ARRANGE
        let data = vec![7_u8; 300];

        // ACT
        let archive = entry(&data);
        let mut slice = archive.as_slice();
        let mut reader = tar::Archive::new(&mut slice);
        let mut entry = reader
            .entries()
            .expect("entries")
            .next()
            .expect("entry")
            .expect("entry ok");
        let mut payload = Vec::new();
        io::copy(&mut entry, &mut payload).expect("copy");

        // ASSERT
        assert_eq!(payload, data);
    }

    #[test]
    fn guard_rejects_a_truncated_stream() {
        // ARRANGE
        let mut member = io::Cursor::new(vec![1_u8, 2, 3]).take(10);
        io::copy(&mut member, &mut io::sink()).expect("copy");

        // ACT
        let result = guard_full(member.limit(), "asset.bin");

        // ASSERT
        let error = result.unwrap_err().to_string();
        assert!(error.contains("7 bytes short"), "error was: {error}");
    }

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
