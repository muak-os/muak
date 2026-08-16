//! Routes known installer files to their streams.

use std::collections::HashMap;
use std::io::Read;

use koci::error::KociError;
use koci::pull;
use koci::pull::entries::MetadataEntry;

use crate::error::{Result, WizardError};
use crate::nodes::{NodeDescriptor, no_dynamic_output_count};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::{Graph, NodeId, PortId};
use crate::pipeline::runtime::{NodePorts, OutputStream};

pub(crate) const STUB: PortId = PortId(0);
pub(crate) const CMDLINE: PortId = PortId(1);
pub(crate) const KERNEL: PortId = PortId(2);
pub(crate) const INITRAMFS: PortId = PortId(3);

pub(crate) const DESCRIPTOR: NodeDescriptor = NodeDescriptor {
    dependencies,
    output_count: no_dynamic_output_count,
    preflight,
    run,
};

/// Source node meaning no dependencies.
fn dependencies(_ctx: &BuildContext<'_, '_, '_>) -> Vec<Dependency> {
    Vec::new()
}

/// Exact tar-entry sizes via the existing koci metadata callback.
fn preflight(graph: &mut Graph, id: NodeId, ctx: &BuildContext<'_, '_, '_>) -> Result<()> {
    let plan = ctx.plan;

    let mut sizes = HashMap::new();
    pull::metadata(
        plan.installer(),
        &plan.arch(),
        None,
        |entry: MetadataEntry| {
            sizes.insert(entry.path, entry.size);
            Ok(())
        },
    )
    .map_err(|e| WizardError::BuildError(format!("extract installer metadata: {e}")))?;

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
            .ok_or_else(|| WizardError::BuildError(format!("missing installer size for {path}")))?;
        let stream = graph.stream_mut(binding.stream)?;
        stream.size = size;
        path.clone_into(&mut stream.name);
    }

    Ok(())
}

/// Pulls the installer once and routes known files to their output streams.
fn run(ports: &mut NodePorts<'_>, ctx: &BuildContext<'_, '_, '_>) -> Result<NodeReport> {
    let plan = ctx.plan;
    let mut outputs: Vec<(PortId, OutputStream)> = ports
        .take_from(STUB, None)?
        .into_iter()
        .map(|(port, endpoint)| Ok((port, endpoint.into_output()?)))
        .collect::<Result<_>>()?;
    let mut seen_stub = false;
    let mut seen_cmdline = false;

    pull::files(plan.installer(), &plan.arch(), None, |entry| {
        route_entry(
            &entry.path,
            entry.reader,
            &mut outputs,
            &mut seen_stub,
            &mut seen_cmdline,
        )
    })
    .map_err(|e| WizardError::BuildError(format!("pull installer files: {e}")))?;

    Ok(NodeReport::Empty)
}

/// Routes one installer entry to its stream, enforcing the entry-order contract.
fn route_entry(
    path: &str,
    reader: &mut dyn Read,
    outputs: &mut [(PortId, OutputStream)],
    seen_stub: &mut bool,
    seen_cmdline: &mut bool,
) -> koci::error::Result<()> {
    match path {
        "stub.efi" => {
            *seen_stub = true;
            copy_optional(reader, outputs, STUB).map_err(KociError::IoError)
        }
        "cmdline" => {
            *seen_cmdline = true;
            copy_optional(reader, outputs, CMDLINE).map_err(KociError::IoError)
        }
        "vmlinuz" if *seen_stub && *seen_cmdline => {
            copy_optional(reader, outputs, KERNEL).map_err(KociError::IoError)
        }
        "vmlinuz" => Err(KociError::IoError(std::io::Error::other(
            "installer entry order: vmlinuz before stub.efi/cmdline",
        ))),
        "initramfs.img" => copy_optional(reader, outputs, INITRAMFS).map_err(KociError::IoError),
        _ => Ok(()),
    }
}

fn file_path(port: PortId) -> Option<&'static str> {
    match port {
        STUB => Some("stub.efi"),
        CMDLINE => Some("cmdline"),
        KERNEL => Some("vmlinuz"),
        INITRAMFS => Some("initramfs.img"),
        _ => None,
    }
}

fn copy_optional(
    reader: &mut dyn Read,
    outputs: &mut [(PortId, OutputStream)],
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
