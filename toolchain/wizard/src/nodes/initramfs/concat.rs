//! Concatenates two ordered initramfs members into one output.

use crate::artifact::Artifact;
use crate::error::{Result, WizardError};
use crate::nodes::initramfs::tail;
use crate::nodes::installer;
use crate::nodes::{NodeDescriptor, NodeKind};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::Graph;
use crate::pipeline::node::{NodeId, PortId};
use crate::pipeline::runtime::NodePorts;

pub(crate) const CONCAT_FIRST: PortId = PortId(0);
pub(crate) const CONCAT_SECOND: PortId = PortId(1);
pub(crate) const CONCAT_OUTPUT: PortId = PortId(2);

pub(crate) const DESCRIPTOR: NodeDescriptor = NodeDescriptor {
    dependencies,
    produces,
    preflight,
    run,
};

/// The raw CPIO tail first, then the compressed installer base.
fn dependencies(_kind: NodeKind, _ctx: &BuildContext<'_, '_>) -> Vec<Dependency> {
    vec![
        Dependency::new(NodeKind::InitramfsTail, tail::TAIL_OUTPUT, CONCAT_FIRST),
        Dependency::new(NodeKind::InstallerPull, installer::INITRAMFS, CONCAT_SECOND),
    ]
}

/// The concatenated initramfs image.
fn produces(_kind: NodeKind, _ctx: &BuildContext<'_, '_>) -> Vec<(PortId, Artifact)> {
    vec![(CONCAT_OUTPUT, Artifact::Initramfs)]
}

/// Concat size = first input + second input.
fn preflight(graph: &mut Graph, id: NodeId, _ctx: &BuildContext<'_, '_>) -> Result<()> {
    let input_size = |port: PortId| -> Result<u64> {
        let sid = graph.node(id)?.input(port)?;
        Ok(graph.stream(sid)?.size)
    };
    let first = input_size(CONCAT_FIRST)?;
    let second = input_size(CONCAT_SECOND)?;
    let output = graph.stream_mut(graph.node(id)?.output(CONCAT_OUTPUT)?)?;
    output.size = first.saturating_add(second);
    "initramfs.img".clone_into(&mut output.name);

    Ok(())
}

/// Emits the first input stream followed by the second into one output.
fn run(
    _kind: NodeKind,
    ports: &mut NodePorts<'_, '_>,
    _ctx: &BuildContext<'_, '_>,
) -> Result<NodeReport> {
    let mut first = ports.input(CONCAT_FIRST)?;
    let mut second = ports.input(CONCAT_SECOND)?;
    let mut output = ports.output(CONCAT_OUTPUT)?;

    std::io::copy(&mut first.reader, &mut output.writer)
        .map_err(|e| WizardError::BuildError(format!("concat stream: {e}")))?;
    std::io::copy(&mut second.reader, &mut output.writer)
        .map_err(|e| WizardError::BuildError(format!("concat stream: {e}")))?;

    Ok(None)
}
