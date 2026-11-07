//! Kernel package pull routed to kernel and cmdline consumers.

mod cmdline;

use std::io::Read;

use koci::error::KociError;
use koci::pull;
use koci::pull::entries::FileEntry;

use crate::artifact::Artifact;
use crate::error::{Result, WizardError};
use crate::nodes::layers::Layer;
use crate::nodes::{NodeDescriptor, NodeKind, measured_sizes};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::Graph;
use crate::pipeline::node::{NodeId, PortId};
use crate::pipeline::runtime::{NodePorts, OutputStream};

pub(crate) const KERNEL: PortId = PortId(0);
pub(crate) const CMDLINE: PortId = PortId(1);

pub(crate) const DESCRIPTOR: NodeDescriptor = NodeDescriptor {
    dependencies,
    produces,
    preflight,
    run,
};

/// The root of the module directory inside the kernel image.
const MODULE_PATH: &str = "lib/modules";

/// `SELinux` file context rules for the kernel modules layer, matching the
/// `fs.cil` `filecon` entries for `/lib/modules`.
const MODULE_FILE_CONTEXTS: &[u8] = b"/lib/modules    system_u:object_r:modules_t:s0\n\
     /lib/modules/.* system_u:object_r:modules_t:s0\n";

/// One layer payload carrying the kernel module files.
const MODULE_LAYER_NAME: &str = "modules";

/// Source node meaning no dependencies.
fn dependencies(_kind: NodeKind, _ctx: &BuildContext<'_, '_>) -> Vec<Dependency> {
    Vec::new()
}

/// The kernel image and the cmdline file.
fn produces(_kind: NodeKind, _ctx: &BuildContext<'_, '_>) -> Vec<(PortId, Artifact)> {
    vec![(KERNEL, Artifact::Kernel), (CMDLINE, Artifact::Cmdline)]
}

/// Exact kernel and cmdline sizes, measured from the image's eStargz TOC.
fn preflight(graph: &mut Graph, id: NodeId, ctx: &BuildContext<'_, '_>) -> Result<()> {
    let source = ctx.build.kernel().source();

    let sizes = measured_sizes(source, ctx.build.arch(), ctx.strategy)?;

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
            .iter()
            .find_map(|entry| (entry.0.as_str() == path).then_some(entry.1))
            .ok_or_else(|| WizardError::BuildError(format!("missing kernel size for {path}")))?;
        let stream = graph.stream_mut(binding.stream)?;
        stream.size = if binding.port == CMDLINE {
            cmdline::size(size, ctx.build.kernel().parameters_append())?
        } else {
            size
        };
        path.clone_into(&mut stream.name);
    }

    Ok(())
}

/// Pulls the kernel package once and routes known files to their output streams.
fn run(
    kind: NodeKind,
    ports: &mut NodePorts<'_, '_>,
    ctx: &BuildContext<'_, '_>,
) -> Result<NodeReport> {
    let source = ctx.build.kernel().source();
    let mut outputs: Vec<(PortId, OutputStream<'_>)> = ports.output_pairs_from(PortId(0), None)?;
    let mut seen_cmdline = false;

    pull::files(
        source,
        &ctx.build.arch(),
        None,
        ctx.strategy,
        &ctx.reporter(kind),
        |mut entry| {
            route_entry(
                &mut entry,
                &mut outputs,
                &mut seen_cmdline,
                ctx.build.kernel().parameters_append(),
            )
        },
    )
    .map_err(|e| WizardError::pull("pull kernel files", source, &e))?;

    Ok(None)
}

/// The kernel modules initramfs layer: only the files under `lib/modules`.
pub(crate) fn module_layer(ctx: &BuildContext<'_, '_>) -> Result<Layer> {
    Ok(Layer {
        name: MODULE_LAYER_NAME.to_owned(),
        source: ctx.build.kernel().source().to_owned(),
        arch: ctx.build.arch(),
        entry: module_entry,
        file_contexts: Some(module_file_contexts()?),
        output_prefix: "",
    })
}

/// Only the kernel module files, never directory entries.
fn module_entry(entry: &FileEntry<'_>) -> bool {
    !entry.path.ends_with('/') && entry.path.starts_with(MODULE_PATH)
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

/// Routes one kernel entry, requiring `cmdline` before `vmlinuz`.
fn route_entry(
    entry: &mut FileEntry<'_>,
    outputs: &mut [(PortId, OutputStream<'_>)],
    seen_cmdline: &mut bool,
    parameters: &[String],
) -> koci::error::Result<()> {
    match entry.path.as_str() {
        "cmdline" => {
            *seen_cmdline = true;
            copy_cmdline(entry, outputs, parameters).map_err(KociError::IoError)
        }
        "vmlinuz" if *seen_cmdline => {
            copy_optional(entry.reader, outputs, KERNEL).map_err(KociError::IoError)
        }
        "vmlinuz" => Err(KociError::IoError(std::io::Error::other(
            "kernel entry order: vmlinuz before cmdline",
        ))),
        _ => Ok(()),
    }
}

/// Appends the profile parameters, keeping the output's preflight size exact.
fn copy_cmdline(
    entry: &mut FileEntry<'_>,
    outputs: &mut [(PortId, OutputStream<'_>)],
    parameters: &[String],
) -> std::io::Result<()> {
    if parameters.is_empty() {
        return copy_optional(entry.reader, outputs, CMDLINE);
    }
    let Some(output) = outputs
        .iter_mut()
        .find_map(|item| (item.0 == CMDLINE).then_some(&mut item.1))
    else {
        return Ok(());
    };
    if cmdline::size(entry.size, parameters)? != output.size {
        return Err(std::io::Error::other(
            "kernel cmdline size differs from preflight",
        ));
    }

    cmdline::write(
        entry.reader,
        &mut output.writer,
        entry.size,
        &parameters.join(" "),
    )
}

/// Copies one routed entry into its output stream, if that stream is live.
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

/// The tar path routed to `port`, when the port carries a file.
fn file_path(port: PortId) -> Option<&'static str> {
    match port {
        KERNEL => Some("vmlinuz"),
        CMDLINE => Some("cmdline"),
        _ => None,
    }
}
