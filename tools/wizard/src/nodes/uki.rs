//! Builds the unified kernel image with yuki.

use std::io::Read as _;

use koci::pull;
use yuki::pe::section::Section;
use yuki::prepare;
use yuki::probe;
use yuki::write::{self, Input};

use crate::SectionInfo;
use crate::error::{Result, WizardError};
use crate::nodes::initramfs;
use crate::nodes::installer;
use crate::nodes::kernel;
use crate::nodes::{NodeDescriptor, NodeKind, no_dynamic_output_count};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::{Graph, NodeId, PortId};
use crate::pipeline::runtime::{InputStream, NodePorts};

pub(crate) const UKI_STUB: PortId = PortId(0);
pub(crate) const UKI_CMDLINE: PortId = PortId(1);
pub(crate) const UKI_KERNEL: PortId = PortId(2);
pub(crate) const UKI_INITRAMFS: PortId = PortId(3);
pub(crate) const UKI_OUTPUT: PortId = PortId(4);

pub(crate) const DESCRIPTOR: NodeDescriptor = NodeDescriptor {
    dependencies,
    output_count: no_dynamic_output_count,
    preflight,
    run,
};

/// Stub and initramfs from the installer and kernel and cmdline from their sources.
fn dependencies(_kind: NodeKind, _ctx: &BuildContext<'_, '_, '_>) -> Vec<Dependency> {
    vec![
        Dependency::fixed(NodeKind::InstallerPull, installer::STUB, UKI_STUB),
        Dependency::fixed(NodeKind::KernelPull, kernel::KERNEL, UKI_KERNEL),
        Dependency::fixed(NodeKind::KernelPull, kernel::CMDLINE, UKI_CMDLINE),
        Dependency::fixed(
            NodeKind::Concat,
            initramfs::concat::CONCAT_OUTPUT,
            UKI_INITRAMFS,
        ),
    ]
}

/// Lower bound on the UKI size in bytes; below this the ESP image cannot be formatted as FAT32.
const MIN_UKI_BYTES: u64 = 32 << 20;

/// Upper bound on the UKI size in bytes; above this the payload exceeds the FAT32 ceiling.
const MAX_UKI_BYTES: u64 = 512 << 20;

/// Probes the bounded stub header prefix and plans the UKI layout to get the size.
fn preflight(graph: &mut Graph, id: NodeId, ctx: &BuildContext<'_, '_, '_>) -> Result<()> {
    let build = ctx.build;

    let mut prefix = Vec::new();
    pull::files(build.installer(), &build.arch(), None, |entry| {
        if entry.path == "stub.efi" {
            let max = u64::try_from(yuki::probe::MAX_HEADER_BYTES)
                .map_err(|e| std::io::Error::other(format!("stub prefix size: {e}")))?;
            entry.reader.take(max).read_to_end(&mut prefix)?;
        }
        Ok(())
    })
    .map_err(|e| WizardError::BuildError(format!("probe stub prefix: {e}")))?;

    let mut stub = prefix.as_slice();
    let probed = probe::probe(&mut stub)
        .map_err(|e| WizardError::BuildError(format!("probe stub header: {e}")))?;

    let input = |port: PortId| -> Result<u64> {
        let sid = graph.node(id)?.input(port)?;
        Ok(graph.stream(sid)?.size)
    };
    let manifest = prepare::prepare(
        probed,
        input(UKI_STUB)?,
        input(UKI_CMDLINE)?,
        input(UKI_KERNEL)?,
        input(UKI_INITRAMFS)?,
    )
    .map_err(|e| WizardError::BuildError(format!("prepare UKI plan: {e}")))?;

    let total_size = manifest.layout().total_size;

    check_bounds(total_size)?;

    let output = graph.stream_mut(graph.node(id)?.output(UKI_OUTPUT)?)?;
    output.size = total_size;
    "uki.efi".clone_into(&mut output.name);

    Ok(())
}

/// Builds the unsigned UKI from the live input streams.
fn run(
    _kind: NodeKind,
    ports: &mut NodePorts<'_>,
    _ctx: &BuildContext<'_, '_, '_>,
) -> Result<NodeReport> {
    let mut stub = ports.take(UKI_STUB)?.into_input()?;
    let mut cmdline = ports.take(UKI_CMDLINE)?.into_input()?;
    let mut kernel = ports.take(UKI_KERNEL)?.into_input()?;
    let mut initramfs = ports.take(UKI_INITRAMFS)?.into_input()?;
    let mut output = ports.take(UKI_OUTPUT)?.into_output()?;

    let probed = probe::probe(&mut stub.reader)
        .map_err(|e| WizardError::BuildError(format!("probe stub header: {e}")))?;
    let manifest = prepare::prepare(probed, stub.size, cmdline.size, kernel.size, initramfs.size)
        .map_err(|e| WizardError::BuildError(format!("prepare UKI plan: {e}")))?;

    let total_size = manifest.layout().total_size;
    if total_size != output.size {
        return Err(WizardError::BuildError(format!(
            "uki size mismatch: runtime {total_size} != preflight {}",
            output.size,
        )));
    }

    write::write(
        &manifest,
        &mut stub.reader,
        input(&mut cmdline),
        input(&mut kernel),
        input(&mut initramfs),
        &mut output.writer,
    )
    .map(|sections| NodeReport::Uki(to_section_infos(sections)))
    .map_err(|e| WizardError::BuildError(format!("uki stream: {e}")))
}

fn input<'b>(stream: &'b mut InputStream<'_>) -> Input<'b> {
    Input {
        reader: &mut stream.reader,
        size: stream.size,
    }
}

fn to_section_infos(sections: Vec<Section>) -> Vec<SectionInfo> {
    sections
        .into_iter()
        .map(|section| SectionInfo {
            name: section.name.to_owned(),
            file_offset: section.file_offset,
            size: section.size,
            hash: section.checksum,
        })
        .collect()
}

fn check_bounds(total_size: u64) -> Result<()> {
    if !(MIN_UKI_BYTES..=MAX_UKI_BYTES).contains(&total_size) {
        return Err(WizardError::BuildError(format!(
            "UKI size {total_size} outside [{MIN_UKI_BYTES}, {MAX_UKI_BYTES}] bytes (FAT32 bounds)"
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_accept_in_range_size() {
        // ARRANGE / ACT
        let result = check_bounds(MIN_UKI_BYTES);
        let upper = check_bounds(MAX_UKI_BYTES);

        // ASSERT
        result.expect("lower bound must pass");
        upper.expect("upper bound must pass");
    }

    #[test]
    fn bounds_reject_undersized_uki() {
        // ARRANGE / ACT
        let result = check_bounds(MIN_UKI_BYTES.saturating_sub(1));

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|e| e.to_string().contains("FAT32 bounds"))
        );
    }

    #[test]
    fn bounds_reject_oversized_uki() {
        // ARRANGE / ACT
        let result = check_bounds(MAX_UKI_BYTES.saturating_add(1));

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|e| e.to_string().contains("FAT32 bounds"))
        );
    }
}
