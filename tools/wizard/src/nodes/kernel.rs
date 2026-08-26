//! Kernel package pull routed to kernel and cmdline consumers.

use std::io::Read;

use koci::error::KociError;
use koci::pull;
use koci::pull::entries::FileEntry;

use crate::error::{Result, WizardError};
use crate::nodes::layers::Layer;
use crate::nodes::{NodeDescriptor, NodeKind, no_dynamic_output_count};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::{Graph, NodeId, PortId};
use crate::pipeline::runtime::{NodePorts, OutputStream};

pub(crate) const KERNEL: PortId = PortId(0);
pub(crate) const CMDLINE: PortId = PortId(1);

pub(crate) const DESCRIPTOR: NodeDescriptor = NodeDescriptor {
    dependencies,
    output_count: no_dynamic_output_count,
    preflight,
    run,
};

/// Source node meaning no dependencies.
fn dependencies(_kind: NodeKind, _ctx: &BuildContext<'_, '_, '_>) -> Vec<Dependency> {
    Vec::new()
}

/// Exact sizes of the kernel and cmdline entries via the koci metadata callback.
fn preflight(graph: &mut Graph, id: NodeId, ctx: &BuildContext<'_, '_, '_>) -> Result<()> {
    let source = ctx.build.kernel().source();

    let mut sizes = std::collections::HashMap::new();
    pull::metadata(source, &ctx.build.arch(), None, |entry| {
        sizes.insert(entry.path, entry.size);
        Ok(())
    })
    .map_err(|e| WizardError::BuildError(format!("extract kernel metadata: {e}")))?;

    let bindings = graph
        .node(id)?
        .output_bindings()
        .copied()
        .collect::<Vec<_>>();
    for binding in bindings {
        let Some(path) = file_path(binding.port) else {
            continue;
        };
        let size = sizes
            .get(path)
            .copied()
            .ok_or_else(|| WizardError::BuildError(format!("missing kernel size for {path}")))?;
        let stream = graph.stream_mut(binding.stream)?;
        stream.size = size;
        path.clone_into(&mut stream.name);
    }

    Ok(())
}

/// Pulls the kernel package once and routes known files to their output streams.
fn run(
    _kind: NodeKind,
    ports: &mut NodePorts<'_>,
    ctx: &BuildContext<'_, '_, '_>,
) -> Result<NodeReport> {
    let source = ctx.build.kernel().source();
    let mut outputs: Vec<(PortId, OutputStream)> = ports
        .take_from(PortId(0), None)?
        .into_iter()
        .map(|(port, endpoint)| Ok((port, endpoint.into_output()?)))
        .collect::<Result<_>>()?;
    let mut seen_cmdline = false;

    pull::files(source, &ctx.build.arch(), None, |entry| {
        route_entry(&entry.path, entry.reader, &mut outputs, &mut seen_cmdline)
    })
    .map_err(|e| WizardError::BuildError(format!("pull kernel files: {e}")))?;

    Ok(NodeReport::Empty)
}

/// Routes one kernel entry, requiring `cmdline` before `vmlinuz`.
fn route_entry(
    path: &str,
    reader: &mut dyn Read,
    outputs: &mut [(PortId, OutputStream<'_>)],
    seen_cmdline: &mut bool,
) -> koci::error::Result<()> {
    match path {
        "cmdline" => {
            *seen_cmdline = true;
            copy_optional(reader, outputs, CMDLINE).map_err(KociError::IoError)
        }
        "vmlinuz" if *seen_cmdline => {
            copy_optional(reader, outputs, KERNEL).map_err(KociError::IoError)
        }
        "vmlinuz" => Err(KociError::IoError(std::io::Error::other(
            "kernel entry order: vmlinuz before cmdline",
        ))),
        _ => Ok(()),
    }
}

fn copy_optional(
    reader: &mut dyn Read,
    outputs: &mut [(PortId, OutputStream<'_>)],
    port: PortId,
) -> std::io::Result<()> {
    let Some(index) = outputs.iter().position(|output| output.0 == port) else {
        return Ok(());
    };
    if let Some(output) = outputs.get_mut(index).map(|item| &mut item.1) {
        std::io::copy(reader, &mut output.writer)?;
    }

    Ok(())
}

fn file_path(port: PortId) -> Option<&'static str> {
    match port {
        KERNEL => Some("vmlinuz"),
        CMDLINE => Some("cmdline"),
        _ => None,
    }
}

/// Payload identity of the kernel modules layer.
const MODULE_LAYER_NAME: &str = "modules";

/// Kernel image path prefix of the module files.
const MODULE_PATH: &str = "lib/modules/";

/// `SELinux` file context rules for the kernel modules layer.
const MODULE_FILE_CONTEXTS: &[u8] = b"/lib/modules    system_u:object_r:modules_t:s0\n\
                                      /lib/modules/.* system_u:object_r:modules_t:s0\n";

/// The kernel modules layer the kernel image contributes to the initramfs.
///
/// # Errors
///
/// Returns an error when the embedded `SELinux` file contexts cannot be parsed.
pub(crate) fn module_layer(ctx: &BuildContext<'_, '_, '_>) -> Result<Layer> {
    Ok(Layer {
        name: MODULE_LAYER_NAME.to_owned(),
        source: ctx.build.kernel().source().to_owned(),
        arch: ctx.build.arch(),
        entry: module_entry,
        file_contexts: Some(module_file_contexts()?),
        output_prefix: "",
    })
}

/// Only the kernel module files.
fn module_entry(entry: &FileEntry<'_>) -> bool {
    entry.path.starts_with(MODULE_PATH)
}

/// Parses the kernel modules `SELinux` file contexts.
///
/// # Errors
///
/// Returns an error when the embedded rules cannot be parsed.
fn module_file_contexts() -> Result<mumi::image::FileContexts> {
    mumi::image::FileContexts::parse(MODULE_FILE_CONTEXTS)
        .map_err(|e| WizardError::BuildError(format!("parse module file contexts: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_entry_accepts_module_files_only() {
        // ARRANGE
        let module_file = FileEntry {
            path: "lib/modules/7.2.0-muak/modules.dep".to_owned(),
            size: 0,
            mode: 0o100_644,
            reader: &mut std::io::empty(),
        };
        let other_file = FileEntry {
            path: "vmlinuz".to_owned(),
            size: 0,
            mode: 0o100_644,
            reader: &mut std::io::empty(),
        };

        // ACT / ASSERT
        assert!(module_entry(&module_file));
        assert!(!module_entry(&other_file));
    }

    #[test]
    fn module_file_contexts_parse() {
        // ARRANGE / ACT
        let contexts = module_file_contexts().expect("parse module file contexts");

        // ASSERT
        drop(contexts);
    }
}
