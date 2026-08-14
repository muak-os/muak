//! Concatenates the base installer initramfs with the CPIO tail to produce a complete initramfs image.

use crate::error::{Result, WizardError};
use crate::nodes::{initramfs::tail, installer};
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::{Graph, NodeId, NodeKind, PortId};
use crate::pipeline::runtime::NodePorts;

pub(crate) const CONCAT_BASE: PortId = PortId(0);
pub(crate) const CONCAT_TAIL: PortId = PortId(1);
pub(crate) const CONCAT_OUTPUT: PortId = PortId(2);

/// The base installer initramfs plus the CPIO tail.
pub(crate) fn dependencies() -> Vec<Dependency> {
    vec![
        Dependency::fixed(NodeKind::InstallerPull, installer::INITRAMFS, CONCAT_BASE),
        Dependency::fixed(NodeKind::InitramfsTail, tail::TAIL_OUTPUT, CONCAT_TAIL),
    ]
}

/// Concat size = base input + tail input.
pub(crate) fn preflight(graph: &mut Graph, id: NodeId) -> Result<()> {
    let input_size = |port: PortId| -> Result<u64> {
        let sid = graph.node(id)?.input(port)?;
        Ok(graph.stream(sid)?.size)
    };
    let base = input_size(CONCAT_BASE)?;
    let tail = input_size(CONCAT_TAIL)?;
    let output = graph.stream_mut(graph.node(id)?.output(CONCAT_OUTPUT)?)?;
    output.size = base.saturating_add(tail);
    "initramfs.img".clone_into(&mut output.name);

    Ok(())
}

/// Emits the first input stream followed by the second into one output.
pub(crate) fn run(ports: &mut NodePorts<'_>) -> Result<NodeReport> {
    let mut first = ports.take(CONCAT_BASE)?.into_input()?;
    let mut second = ports.take(CONCAT_TAIL)?.into_input()?;
    let mut output = ports.take(CONCAT_OUTPUT)?.into_output()?;

    std::io::copy(&mut first.reader, &mut output.writer)
        .map_err(|e| WizardError::BuildError(format!("concat stream: {e}")))?;
    std::io::copy(&mut second.reader, &mut output.writer)
        .map_err(|e| WizardError::BuildError(format!("concat stream: {e}")))?;

    Ok(NodeReport::Empty)
}
