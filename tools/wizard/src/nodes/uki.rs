//! Builds the unified kernel image with yuki.

use std::io::Read as _;

use koci::pull;
use yuki::pe::section::Section;
use yuki::prepare;
use yuki::probe;
use yuki::write::{self, Input};

use crate::SectionInfo;
use crate::error::{Result, WizardError};
use crate::nodes::{initramfs, installer};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::{Graph, NodeId, NodeKind, PortId};
use crate::pipeline::runtime::{DynWriter, InputStream, NodePorts};

pub(crate) const UKI_STUB: PortId = PortId(0);
pub(crate) const UKI_CMDLINE: PortId = PortId(1);
pub(crate) const UKI_KERNEL: PortId = PortId(2);
pub(crate) const UKI_INITRAMFS: PortId = PortId(3);
pub(crate) const UKI_OUTPUT: PortId = PortId(4);

/// Stub, cmdline, and kernel from the installer and complete initramfs.
pub(crate) fn dependencies() -> Vec<Dependency> {
    vec![
        Dependency::fixed(NodeKind::InstallerPull, installer::STUB, UKI_STUB),
        Dependency::fixed(NodeKind::InstallerPull, installer::CMDLINE, UKI_CMDLINE),
        Dependency::fixed(NodeKind::InstallerPull, installer::KERNEL, UKI_KERNEL),
        Dependency::fixed(NodeKind::Concat, initramfs::CONCAT_OUTPUT, UKI_INITRAMFS),
    ]
}

/// Lower bound on the UKI size in bytes; below this the ESP image cannot be formatted as FAT32.
const MIN_UKI_BYTES: u64 = 32 << 20;

/// Upper bound on the UKI size in bytes; above this the payload exceeds the FAT32 ceiling.
const MAX_UKI_BYTES: u64 = 512 << 20;

/// Probes the bounded stub header prefix through the koci file callback and
/// plans the UKI layout, so the output size is known before any pipe exists.
pub(crate) async fn preflight(
    graph: &mut Graph,
    id: NodeId,
    context: &BuildContext<'_, '_, '_>,
) -> Result<()> {
    let plan = context.plan;

    let mut prefix = Vec::new();
    pull::files(plan.installer(), &plan.arch(), None, |entry| {
        if entry.path == "stub.efi" {
            let max = u64::try_from(yuki::probe::MAX_HEADER_BYTES)
                .map_err(|e| std::io::Error::other(format!("stub prefix size: {e}")))?;
            entry.reader.take(max).read_to_end(&mut prefix)?;
        }
        Ok(())
    })
    .await
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
        None,
    )
    .map_err(|e| WizardError::BuildError(format!("prepare UKI plan: {e}")))?;

    let total_size = manifest.layout().total_size;

    if !(MIN_UKI_BYTES..=MAX_UKI_BYTES).contains(&total_size) {
        return Err(WizardError::BuildError(format!(
            "UKI size {total_size} outside [{MIN_UKI_BYTES}, {MAX_UKI_BYTES}] bytes (FAT32 bounds)"
        )));
    }

    graph.stream_mut(graph.node(id)?.output(UKI_OUTPUT)?)?.size = total_size;

    Ok(())
}

/// Builds the unsigned UKI from the live input streams.
pub(crate) fn run(_ctx: &BuildContext<'_, '_, '_>, ports: &mut NodePorts) -> Result<NodeReport> {
    let mut stub = ports.take(UKI_STUB)?.into_input()?;
    let mut cmdline = ports.take(UKI_CMDLINE)?.into_input()?;
    let mut kernel = ports.take(UKI_KERNEL)?.into_input()?;
    let mut initramfs = ports.take(UKI_INITRAMFS)?.into_input()?;
    let mut output = ports.take(UKI_OUTPUT)?.into_output()?;

    let probed = probe::probe(&mut stub.reader)
        .map_err(|e| WizardError::BuildError(format!("probe stub header: {e}")))?;
    let manifest = prepare::prepare(
        probed,
        stub.size,
        cmdline.size,
        kernel.size,
        initramfs.size,
        None,
    )
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
        None,
        input(&mut kernel),
        input(&mut initramfs),
        &mut DynWriter::new(&mut output.writer),
    )
    .map(|sections| NodeReport::Uki(to_section_infos(sections)))
    .map_err(|e| WizardError::BuildError(format!("uki stream: {e}")))
}

fn input(stream: &mut InputStream) -> Input<'_> {
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
