//! Overlay pull.

use std::collections::HashMap;
use std::io::{self, Read};

use koci::error::KociError;
use koci::progress::Noop;
use koci::pull;
use koci::pull::demux::Demux;
use koci::pull::session::Session;

use crate::artifact::Artifact;
use crate::domain::overlay::{Asset, entry_name};
use crate::domain::resolution::Overlay;
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

/// Pulls the overlay source over one session.
///
/// A validation walk first checks the sizes annotation, asset presence, and
/// canonical (sorted) entry order; canonically sorted images are then served
/// by a single demux pass, while unsorted ones fall back to per-asset walks
/// that keep every stream's write order canonical and deadlock-free.
fn run(
    _kind: NodeKind,
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
    let mut outputs = ports.outputs_from(PULL_OUTPUTS_FIRST, None)?;
    let session = pull::session::open(&overlay.source, &overlay.arch, None)
        .map_err(|e| WizardError::BuildError(format!("open overlay session: {e}")))?;

    if discover(&session, overlay, assets)? {
        demux_pass(&session, assets, outputs)?;
    } else {
        asset_walks(&session, overlay, &mut outputs)?;
    }

    Ok(None)
}

/// One walk validating the annotation, asset presence, and canonical order.
///
/// Returns whether the image's asset entries arrive in canonical (sorted)
/// order: only then can one demux pass feed the slot pipes without a pipe
/// filling ahead of its canonical-position consumer.
fn discover(session: &Session, overlay: &Overlay, assets: &[Asset]) -> Result<bool> {
    let sizes = assets
        .iter()
        .map(|asset| (asset.name(), asset.size()))
        .collect::<HashMap<&str, u64>>();
    let slots = assets
        .iter()
        .enumerate()
        .map(|(index, asset)| (asset.name(), index))
        .collect::<HashMap<&str, usize>>();
    let mut present = vec![false; assets.len()];
    let mut canonical = true;
    let mut last = 0_usize;
    let mut failure = None;

    session
        .walk(&Noop, |entry| {
            if failure.is_some() {
                return Ok(());
            }
            let Some(name) = entry_name(overlay, &entry.path) else {
                return Ok(());
            };
            let Some(&slot) = slots.get(name.as_str()) else {
                return Ok(());
            };
            if let Some(expected) = sizes.get(name.as_str())
                && *expected != entry.size
            {
                failure = Some(WizardError::BuildError(format!(
                    "overlay asset {name}: annotated {expected} bytes, image holds {}",
                    entry.size
                )));
                return Ok(());
            }
            if slot < last {
                canonical = false;
            }
            last = slot;
            if let Some(seen) = present.get_mut(slot) {
                *seen = true;
            }

            Ok(())
        })
        .map_err(|e| WizardError::BuildError(format!("validate overlay image: {e}")))?;
    if let Some(failure) = failure {
        return Err(failure);
    }

    let missing = assets
        .iter()
        .zip(present.iter().map(|seen| !seen))
        .filter_map(|(asset, absent)| absent.then_some(asset.name()))
        .collect::<Vec<_>>()
        .join(", ");
    if missing.is_empty() {
        return Ok(canonical);
    }

    Err(WizardError::BuildError(format!(
        "overlay source is missing assets: {missing}"
    )))
}

/// Single-pass demux: routes every entry to its output stream in one walk.
fn demux_pass(
    session: &Session,
    assets: &[Asset],
    outputs: Vec<OutputStream<'_, '_>>,
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
        .walk(&Noop, |entry| demux.route(entry))
        .map_err(|e| WizardError::BuildError(format!("demux overlay files: {e}")))
}

/// Per-asset walks for images whose entries are not canonically sorted.
///
/// Each stream is written in canonical order, matching the consumers' read
/// order, so the pipes cannot deadlock; the walks are served from the koci
/// blob cache.
fn asset_walks(
    session: &Session,
    overlay: &Overlay,
    outputs: &mut [OutputStream<'_, '_>],
) -> Result<()> {
    for output in outputs {
        let name = output.name;
        let sink = &mut *output;
        session
            .walk(&Noop, |entry| {
                copy_when_matched(&entry.path, entry.reader, overlay, name, sink)
                    .map_err(KociError::IoError)
            })
            .map_err(|e| WizardError::BuildError(format!("pull overlay files: {e}")))?;
    }

    Ok(())
}

fn copy_when_matched(
    path: &str,
    reader: &mut dyn Read,
    overlay: &Overlay,
    expected: &str,
    output: &mut OutputStream<'_, '_>,
) -> io::Result<()> {
    if entry_name(overlay, path).is_some_and(|found| found == expected) {
        io::copy(reader, &mut output.writer).map(|_| ())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::runtime::OutputWriter;

    fn output<'name>(name: &'name str, sink: &'name mut Vec<u8>) -> OutputStream<'name, 'name> {
        OutputStream {
            name,
            size: 0,
            writer: OutputWriter::Target(sink),
        }
    }

    fn overlay() -> Overlay {
        Overlay::new(
            "board".to_owned(),
            "board".to_owned(),
            "ghcr.io/example/board:latest".to_owned(),
            koci::arch::Arch::Arm64,
        )
    }

    #[test]
    fn copies_only_the_matching_entry() {
        // ARRANGE
        let mut sink = Vec::new();
        let mut stream = output("a.txt", &mut sink);
        let ov = overlay();
        let mut data = io::Cursor::new(b"payload".to_vec());
        let esp = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B";

        // ACT
        copy_when_matched(
            &format!("board/partitions/{esp}/a.txt"),
            &mut data,
            &ov,
            "a.txt",
            &mut stream,
        )
        .expect("copy matching entry");

        // ASSERT
        assert_eq!(&sink, b"payload");
    }

    #[test]
    fn skips_non_asset_and_mismatched_entries() {
        // ARRANGE
        let mut sink = Vec::new();
        let mut stream = output("a.txt", &mut sink);
        let ov = overlay();
        let mut data = io::empty();
        let esp = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B";

        // ACT
        copy_when_matched("board/stray.txt", &mut data, &ov, "a.txt", &mut stream)
            .expect("skip stray");
        copy_when_matched(
            &format!("board/partitions/{esp}/other.bin"),
            &mut data,
            &ov,
            "a.txt",
            &mut stream,
        )
        .expect("skip mismatch");

        // ASSERT
        assert_eq!(sink, b"");
    }
}
