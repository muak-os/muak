//! Streams pre-planned opaque extension payloads.

use koci::arch::Arch;
use koci::error::KociError;
use koci::pull;
use koci::pull::entries::FileEntry;

use crate::domain::resolution::Extension;
use crate::error::{Result, WizardError};
use crate::nodes::{NodeDescriptor, NodeKind};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::{Graph, NodeId, PortId};
use crate::pipeline::runtime::{Endpoint, NodePorts};

pub(crate) const FIRST_OUTPUT: PortId = PortId(0);

pub(crate) const DESCRIPTOR: NodeDescriptor = NodeDescriptor {
    dependencies,
    output_count,
    preflight: preflight_stub,
    run: run_stub,
};

/// Source node meaning no dependencies.
fn dependencies(_kind: NodeKind, _ctx: &BuildContext<'_, '_, '_>) -> Vec<Dependency> {
    Vec::new()
}

/// Pulls and plans the extension payloads exactly once, returning the
/// `Planned` list in canonical source order for the other nodes.
pub(crate) fn preflight(
    graph: &mut Graph,
    id: NodeId,
    ctx: &BuildContext<'_, '_, '_>,
) -> Result<Vec<mumi::payload::Planned>> {
    let build = ctx.build;

    let mut payloads = pull(build.extensions(), build.arch())?;
    let planned = mumi::payload::plan(&mut payloads, &config())
        .map_err(|e| WizardError::BuildError(format!("plan extension payloads: {e}")))?;

    let bindings = graph
        .node(id)?
        .output_bindings()
        .copied()
        .collect::<Vec<_>>();
    if bindings.len() != planned.len() {
        return Err(WizardError::BuildError(format!(
            "extension output/payload count mismatch: {} != {}",
            bindings.len(),
            planned.len(),
        )));
    }
    for (binding, payload) in bindings.iter().zip(&planned) {
        let meta = payload.meta();
        let stream = graph.stream_mut(binding.stream)?;
        stream.size = meta.size;
        stream.name = format!("extensions/{}{}", meta.name.replace('/', "-"), meta.format);
    }

    Ok(planned)
}

/// Streams each planned payload into its output stream without re-pulling
/// or re-planning. The payload format is opaque to wizard.
pub(crate) fn run(
    payloads: &[mumi::payload::Planned],
    ports: &mut NodePorts<'_>,
) -> Result<NodeReport> {
    let mut outputs = Endpoint::into_outputs(
        ports
            .take_from(FIRST_OUTPUT, Some(payloads.len()))?
            .into_iter()
            .map(|(_, endpoint)| endpoint),
    )?;
    for (payload, output) in payloads.iter().zip(outputs.iter_mut()) {
        payload
            .write(&mut output.writer)
            .map_err(|e| WizardError::BuildError(format!("stream extension payload: {e}")))?;
    }

    Ok(NodeReport::Empty)
}

fn pull(extensions: &[Extension], arch: Arch) -> Result<Vec<mumi::payload::Payload>> {
    let mut payloads = Vec::with_capacity(extensions.len());

    for ext in extensions {
        let mut payload = mumi::payload::Payload::new(ext.name());
        pull::files(ext.source(), &arch, None, |entry| {
            add_entry(&mut payload, entry)
        })
        .map_err(|e| WizardError::BuildError(format!("pull extension {}: {e}", ext.source())))?;
        payloads.push(payload);
    }

    Ok(payloads)
}

fn add_entry(
    payload: &mut mumi::payload::Payload,
    entry: FileEntry<'_>,
) -> koci::error::Result<()> {
    let path = entry.path.clone();
    let reader = entry.reader;
    let file = mumi::payload::FileEntry {
        path: format!("/{path}"),
        size: entry.size,
        mode: 0o100_000 | entry.mode,
    };
    payload.add_file(file, reader).map_err(|e| {
        KociError::IoError(std::io::Error::other(format!(
            "add extension file {path}: {e}"
        )))
    })
}

/// One payload stream per extension, in canonical source order.
fn output_count(ctx: &BuildContext<'_, '_, '_>) -> Result<usize> {
    let count = ctx.build.extensions().len();
    if count == usize::MAX {
        return Err(WizardError::BuildError(
            "extension count overflow".to_owned(),
        ));
    }
    Ok(count)
}

/// Table slot for `preflight`; the real preflight returns the payload list
/// and is called from `pipeline::preflight` directly until extension payloads
/// become re-derivable.
fn preflight_stub(_graph: &mut Graph, _id: NodeId, _ctx: &BuildContext<'_, '_, '_>) -> Result<()> {
    Err(WizardError::BuildError(
        "extension payloads are handled outside the descriptor table".to_owned(),
    ))
}

/// The payload-carrying run is dispatched by the executor directly until extension payloads become re-derivable.
fn run_stub(
    _kind: NodeKind,
    _ports: &mut NodePorts<'_>,
    _ctx: &BuildContext<'_, '_, '_>,
) -> Result<NodeReport> {
    Err(WizardError::BuildError(
        "extension payloads are handled outside the descriptor table".to_owned(),
    ))
}

fn config() -> mumi::image::BuildConfig {
    mumi::image::BuildConfig {
        compression_level: mumi::DEFAULT_ZSTD_COMPRESSION_LEVEL,
        file_contexts: None,
    }
}
