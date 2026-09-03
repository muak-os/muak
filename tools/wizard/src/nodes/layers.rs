//! Layer payload nodes for the kernel modules and extension initramfs layers.

use koci::arch::Arch;
use koci::error::KociError;
use koci::pull;
use koci::pull::entries::FileEntry;

use crate::artifact::Artifact;
use crate::domain::resolution::Extension;
use crate::error::{Result, WizardError};
use crate::nodes::kernel;
use crate::nodes::{NodeDescriptor, NodeKind};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::{Graph, NodeId, PortId};
use crate::pipeline::runtime::{Endpoint, NodePorts, OutputStream};

pub(crate) const FIRST_OUTPUT: PortId = PortId(0);

pub(crate) const DESCRIPTOR: NodeDescriptor = NodeDescriptor {
    dependencies,
    produces,
    preflight,
    run,
};

/// One layer payload.
pub(crate) struct Layer {
    pub(crate) name: String,
    pub(crate) source: String,
    pub(crate) arch: Arch,
    pub(crate) entry: fn(&FileEntry<'_>) -> bool,
    pub(crate) file_contexts: Option<mumi::image::FileContexts>,
    pub(crate) output_prefix: &'static str,
}

/// Source node meaning no dependencies.
fn dependencies(_kind: NodeKind, _ctx: &BuildContext<'_, '_>) -> Vec<Dependency> {
    Vec::new()
}

/// Layer payloads are initramfs members, never requested artifacts.
fn produces(_kind: NodeKind, _ctx: &BuildContext<'_, '_>) -> Vec<(PortId, Artifact)> {
    Vec::new()
}

/// Pulls and measures every layer payload once, recording stream size and name.
pub(crate) fn preflight(graph: &mut Graph, id: NodeId, ctx: &BuildContext<'_, '_>) -> Result<()> {
    let layers = layer_specs(ctx)?;
    let mut payloads = pull_payloads(&layers)?;
    let planned = measure(&mut payloads, &layers)?;

    let bindings = graph
        .node(id)?
        .output_bindings()
        .copied()
        .collect::<Vec<_>>();
    if bindings.len() != planned.len() {
        return Err(WizardError::BuildError(format!(
            "layer output/payload count mismatch: {} != {}",
            bindings.len(),
            planned.len(),
        )));
    }
    for (binding, (layer, payload)) in bindings.iter().zip(layers.iter().zip(&planned)) {
        let meta = payload.meta();
        let stream = graph.stream_mut(binding.stream)?;
        stream.size = meta.size;
        stream.name = stream_name(layer, meta);
    }

    Ok(())
}

/// Re-pulls and remeasures every layer payload, then streams it into its output stream.
pub(crate) fn run(
    _kind: NodeKind,
    ports: &mut NodePorts<'_, '_>,
    ctx: &BuildContext<'_, '_>,
) -> Result<NodeReport> {
    let layers = layer_specs(ctx)?;
    let mut payloads = pull_payloads(&layers)?;
    let planned = measure(&mut payloads, &layers)?;

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
            .map_err(|e| WizardError::BuildError(format!("stream layer payload: {e}")))?;
    }

    Ok(NodeReport::Empty)
}

fn layer_specs(ctx: &BuildContext<'_, '_>) -> Result<Vec<Layer>> {
    let mut layers = Vec::with_capacity(ctx.build.payload_layer_count());
    layers.push(kernel::module_layer(ctx)?);
    for extension in ctx.build.extensions() {
        layers.push(extension_layer(extension, ctx.build.arch()));
    }

    Ok(layers)
}

fn extension_layer(extension: &Extension, arch: Arch) -> Layer {
    Layer {
        name: extension.name().to_owned(),
        source: extension.source().to_owned(),
        arch,
        entry: include_all,
        file_contexts: None,
        output_prefix: "extensions/",
    }
}

fn include_all(_entry: &FileEntry<'_>) -> bool {
    true
}

fn stream_name(layer: &Layer, meta: &mumi::payload::Meta) -> String {
    format!(
        "{}{}{}",
        layer.output_prefix,
        meta.name.replace('/', "-"),
        meta.format
    )
}

fn ensure_size_matches(
    planned: &mumi::payload::Planned,
    output: &OutputStream<'_, '_>,
) -> Result<()> {
    if planned.size() != output.size {
        return Err(WizardError::BuildError(format!(
            "layer payload size drift: measured {} but promised {}",
            planned.size(),
            output.size,
        )));
    }

    Ok(())
}

fn measure(
    payloads: &mut [mumi::payload::Payload],
    layers: &[Layer],
) -> Result<Vec<mumi::payload::Planned>> {
    let mut planned = Vec::with_capacity(payloads.len());
    for (payload, layer) in payloads.iter_mut().zip(layers) {
        let config = build_config(layer.file_contexts.clone());
        let mut one =
            mumi::payload::plan(core::slice::from_mut(payload), &config).map_err(|e| {
                WizardError::BuildError(format!("plan layer payload {}: {e}", layer.name))
            })?;
        let payload = one.pop().ok_or_else(|| {
            WizardError::BuildError(format!("empty plan result for layer {}", layer.name))
        })?;
        planned.push(payload);
    }

    Ok(planned)
}

fn pull_payloads(layers: &[Layer]) -> Result<Vec<mumi::payload::Payload>> {
    let mut payloads = Vec::with_capacity(layers.len());
    for layer in layers {
        let mut payload = mumi::payload::Payload::new(layer.name.clone());
        pull::files(&layer.source, &layer.arch, None, |entry| {
            add_entry(&mut payload, entry, layer.entry)
        })
        .map_err(|e| WizardError::BuildError(format!("pull layer {}: {e}", layer.name)))?;
        payloads.push(payload);
    }

    Ok(payloads)
}

fn add_entry(
    payload: &mut mumi::payload::Payload,
    entry: FileEntry<'_>,
    include: fn(&FileEntry<'_>) -> bool,
) -> koci::error::Result<()> {
    if !include(&entry) {
        return Ok(());
    }
    let path = entry.path.clone();
    let reader = entry.reader;
    let file = mumi::payload::FileEntry {
        path: format!("/{path}"),
        size: entry.size,
        mode: 0o100_000 | entry.mode,
    };

    payload.add_file(file, reader).map_err(|e| {
        KociError::IoError(std::io::Error::other(format!("add layer file {path}: {e}")))
    })
}

fn build_config(file_contexts: Option<mumi::image::FileContexts>) -> mumi::image::BuildConfig {
    mumi::image::BuildConfig {
        compression_level: mumi::DEFAULT_ZSTD_COMPRESSION_LEVEL,
        file_contexts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::runtime::{OutputStream, OutputWriter};

    fn planned(size: u64) -> mumi::payload::Planned {
        let mut payload = mumi::payload::Payload::new("muak-test/layer");
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

    fn output(name: &'static str, size: u64) -> OutputStream<'static, 'static> {
        let (writer, _reader) = std::os::unix::net::UnixStream::pair().expect("pipe pair");
        OutputStream {
            name,
            size,
            writer: OutputWriter::Pipe(writer),
        }
    }

    #[test]
    fn size_guard_accepts_matching_plan() {
        // ARRANGE
        let planned = planned(4096);
        let out = output("modules.erofs", planned.size());

        // ACT
        let result = ensure_size_matches(&planned, &out);

        // ASSERT
        result.expect("matching sizes must pass the drift guard");
    }

    #[test]
    fn size_guard_rejects_promise_drift() {
        // ARRANGE
        let planned = planned(4096);
        let out = output("modules.erofs", planned.size().saturating_add(1));

        // ACT
        let result = ensure_size_matches(&planned, &out);

        // ASSERT
        assert!(
            matches!(result, Err(WizardError::BuildError(detail)) if detail.contains("size drift"))
        );
    }

    #[test]
    fn stream_names_place_root_layers_at_root_and_extensions_in_extensions() {
        // ARRANGE
        let module_layer = Layer {
            name: "modules".to_owned(),
            source: String::new(),
            arch: Arch::Amd64,
            entry: include_all,
            file_contexts: None,
            output_prefix: "",
        };
        let extension_layer = extension_layer(
            &Extension::new(
                "muak-os/qemu".to_owned(),
                "ghcr.io/muak-os/pkgs/qemu:latest".to_owned(),
            ),
            Arch::Amd64,
        );
        let module_meta = mumi::payload::Meta {
            name: "modules".to_owned(),
            format: ".erofs".to_owned(),
            size: 0,
        };
        let extension_meta = mumi::payload::Meta {
            name: "muak-os/qemu".to_owned(),
            format: ".erofs".to_owned(),
            size: 0,
        };

        // ACT / ASSERT
        assert_eq!(stream_name(&module_layer, &module_meta), "modules.erofs");
        assert_eq!(
            stream_name(&extension_layer, &extension_meta),
            "extensions/muak-os-qemu.erofs"
        );
    }
}
