//! Extension payload nodes, re-derived from source names at run time.

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
use crate::pipeline::runtime::{Endpoint, NodePorts, OutputStream};

pub(crate) const FIRST_OUTPUT: PortId = PortId(0);

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

/// Pulls and measures each extension payload once, recording stream size and name.
pub(crate) fn preflight(
    graph: &mut Graph,
    id: NodeId,
    ctx: &BuildContext<'_, '_, '_>,
) -> Result<()> {
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

    Ok(())
}

/// Re-pulls and remeasures each extension payload, then streams it into its output stream.
pub(crate) fn run(
    _kind: NodeKind,
    ports: &mut NodePorts<'_>,
    ctx: &BuildContext<'_, '_, '_>,
) -> Result<NodeReport> {
    let build = ctx.build;

    let mut payloads = pull(build.extensions(), build.arch())?;
    let planned = mumi::payload::plan(&mut payloads, &config())
        .map_err(|e| WizardError::BuildError(format!("re-plan extension payloads: {e}")))?;

    let mut outputs = Endpoint::into_outputs(
        ports
            .take_from(FIRST_OUTPUT, Some(planned.len()))?
            .into_iter()
            .map(|(_, endpoint)| endpoint),
    )?;

    for ((payload, output), source) in planned.iter().zip(outputs.iter_mut()).zip(&payloads) {
        ensure_size_matches(payload, output)?;
        payload
            .write(&mut output.writer, source)
            .map_err(|e| WizardError::BuildError(format!("stream extension payload: {e}")))?;
    }

    Ok(NodeReport::Empty)
}

/// The preflight promise must equal what run re-derives.
fn ensure_size_matches(planned: &mumi::payload::Planned, output: &OutputStream<'_>) -> Result<()> {
    if planned.size() != output.size {
        return Err(WizardError::BuildError(format!(
            "extension payload size drift: measured {} but promised {}",
            planned.size(),
            output.size,
        )));
    }

    Ok(())
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

fn config() -> mumi::image::BuildConfig {
    mumi::image::BuildConfig {
        compression_level: mumi::DEFAULT_ZSTD_COMPRESSION_LEVEL,
        file_contexts: None,
    }
}

#[cfg(test)]
mod tests {
    use super::ensure_size_matches;
    use crate::error::WizardError;
    use crate::pipeline::runtime::OutputStream;

    fn planned(size: u64) -> mumi::payload::Planned {
        let mut payload = mumi::payload::Payload::new("muak-test/extension");
        let content = vec![b'x'; usize::try_from(size).unwrap_or(0)];
        payload
            .add_file(
                mumi::payload::FileEntry {
                    path: "/usr/bin/tool".to_owned(),
                    size: u64::try_from(content.len()).unwrap_or(0),
                    mode: 0o100_755,
                },
                &mut std::io::Cursor::new(content),
            )
            .expect("add payload file");
        mumi::payload::plan(
            &mut [payload],
            &mumi::image::BuildConfig {
                compression_level: mumi::DEFAULT_ZSTD_COMPRESSION_LEVEL,
                file_contexts: None,
            },
        )
        .expect("plan payload")
        .remove(0)
    }

    fn output(name: &'static str, size: u64) -> OutputStream<'static> {
        let (writer, _reader) = std::os::unix::net::UnixStream::pair().expect("pipe pair");
        OutputStream { name, size, writer }
    }

    #[test]
    fn size_guard_accepts_matching_plan() {
        // ARRANGE
        let planned = planned(4096);
        let out = output("extensions/muak-test-extension.erofs", planned.size());

        // ACT
        let result = ensure_size_matches(&planned, &out);

        // ASSERT
        result.expect("matching sizes must pass the drift guard");
    }

    #[test]
    fn size_guard_rejects_promise_drift() {
        // ARRANGE
        let planned = planned(4096);
        let out = output(
            "extensions/muak-test-extension.erofs",
            planned.size().saturating_add(1),
        );

        // ACT
        let result = ensure_size_matches(&planned, &out);

        // ASSERT
        assert!(
            matches!(result, Err(WizardError::BuildError(detail)) if detail.contains("size drift"))
        );
    }
}
