//! The initramfs CPIO tail builder and the Concat composition node.

use std::io::Read;

use ramune::Entry;

use crate::error::{Result, WizardError};
use crate::nodes::{extensions, installer};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::{Graph, NodeId, NodeKind, PortId};
use crate::pipeline::runtime::{DynWriter, Endpoint, NodePorts};

pub(crate) const TAIL_OUTPUT: PortId = PortId(0);
pub(crate) const TAIL_INPUTS_FIRST: PortId = PortId(1);
pub(crate) const CONCAT_BASE: PortId = PortId(0);
pub(crate) const CONCAT_TAIL: PortId = PortId(1);
pub(crate) const CONCAT_OUTPUT: PortId = PortId(2);

/// One extension payload stream per extension, in canonical source order.
pub(crate) fn tail_dependencies() -> Vec<Dependency> {
    vec![Dependency::many(
        NodeKind::ExtensionPayloads,
        extensions::FIRST_OUTPUT,
        TAIL_INPUTS_FIRST,
    )]
}

/// The base installer initramfs plus the CPIO tail.
pub(crate) fn concat_dependencies() -> Vec<Dependency> {
    vec![
        Dependency::fixed(NodeKind::InstallerPull, installer::INITRAMFS, CONCAT_BASE),
        Dependency::fixed(NodeKind::InitramfsTail, TAIL_OUTPUT, CONCAT_TAIL),
    ]
}

/// Exact CPIO tail size.
pub(crate) fn preflight_tail(
    graph: &mut Graph,
    id: NodeId,
    context: &BuildContext<'_, '_, '_>,
    planned: &[mumi::payload::Planned],
) -> Result<()> {
    let mut entries = Vec::with_capacity(planned.len().saturating_add(1));
    for payload in planned {
        let meta = payload.meta();
        entries.push(Entry {
            path: format!("extensions/{}{}", meta.name.replace('/', "-"), meta.format),
            mode: 0o100_644,
            len: meta.size,
        });
    }
    let profile_len = u64::try_from(context.profile.len())
        .map_err(|e| WizardError::BuildError(format!("profile size overflow: {e}")))?;
    if !context.profile.is_empty() {
        entries.push(profile_entry(profile_len));
    }
    let tail = ramune::archive::size(&entries);

    let bindings = graph
        .node(id)?
        .output_bindings()
        .copied()
        .collect::<Vec<_>>();
    for binding in bindings {
        graph.stream_mut(binding.stream)?.size = tail;
    }

    Ok(())
}

/// Concat size = base input + tail input, found by the node-local port constants shared with the Concat runner.
pub(crate) fn preflight_concat(graph: &mut Graph, id: NodeId) -> Result<()> {
    let input_size = |port: PortId| -> Result<u64> {
        let sid = graph.node(id)?.input(port)?;
        Ok(graph.stream(sid)?.size)
    };
    let base = input_size(CONCAT_BASE)?;
    let tail = input_size(CONCAT_TAIL)?;
    let output = graph.node(id)?.output(CONCAT_OUTPUT)?;
    graph.stream_mut(output)?.size = base.saturating_add(tail);

    Ok(())
}

/// Streams one CPIO entry per extension payload stream plus the profile entry in canonical order.
pub(crate) fn run_tail(
    ctx: &BuildContext<'_, '_, '_>,
    planned: &[mumi::payload::Planned],
    ports: &mut NodePorts,
) -> Result<NodeReport> {
    let mut inputs = Endpoint::into_inputs(
        ports
            .take_from(TAIL_INPUTS_FIRST, Some(planned.len()))?
            .into_iter()
            .map(|(_, endpoint)| endpoint),
    )?;

    let mut pairs: Vec<(Entry, &mut dyn Read)> = Vec::with_capacity(inputs.len().saturating_add(1));
    for (input, payload) in inputs.iter_mut().zip(planned) {
        let meta = payload.meta();
        let reader: &mut dyn Read = &mut input.reader;
        pairs.push((
            Entry {
                path: format!("extensions/{}{}", meta.name.replace('/', "-"), meta.format),
                mode: 0o100_644,
                len: input.size,
            },
            reader,
        ));
    }
    let mut profile: &[u8] = ctx.profile;
    if !profile.is_empty() {
        let len = u64::try_from(ctx.profile.len())
            .map_err(|e| WizardError::BuildError(format!("profile size overflow: {e}")))?;
        let reader: &mut dyn Read = &mut profile;
        pairs.push((profile_entry(len), reader));
    }

    let mut output = ports.take(TAIL_OUTPUT)?.into_output()?;
    ramune::archive::cpio(&mut pairs, &mut DynWriter::new(&mut output.writer))
        .map_err(|e| WizardError::BuildError(format!("build initramfs tail: {e}")))?;

    Ok(NodeReport::Empty)
}

/// Emits the first input stream followed by the second into one output.
pub(crate) fn run_concat(ports: &mut NodePorts) -> Result<NodeReport> {
    let mut first = ports.take(CONCAT_BASE)?.into_input()?;
    let mut second = ports.take(CONCAT_TAIL)?.into_input()?;
    let mut output = ports.take(CONCAT_OUTPUT)?.into_output()?;

    std::io::copy(&mut first.reader, &mut output.writer)
        .map_err(|e| WizardError::BuildError(format!("concat stream: {e}")))?;
    std::io::copy(&mut second.reader, &mut output.writer)
        .map_err(|e| WizardError::BuildError(format!("concat stream: {e}")))?;

    Ok(NodeReport::Empty)
}

#[must_use]
fn profile_entry(len: u64) -> Entry {
    Entry {
        path: "profile.toml".to_owned(),
        mode: 0o100_644,
        len,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_stable_archive_names() {
        // ARRANGE / ACT / ASSERT
        assert_eq!("muak-os/qemu".replace('/', "-"), "muak-os-qemu");
        assert_eq!(
            format!(
                "extensions/{}{}",
                "muak-os/qemu".replace('/', "-"),
                ".erofs"
            ),
            "extensions/muak-os-qemu.erofs"
        );
    }

    #[test]
    fn tail_exact_size_matches_cpio_output() {
        // ARRANGE
        let entry = Entry {
            path: "a".into(),
            mode: 0o100_644,
            len: 5,
        };
        let expected = ramune::archive::size(core::slice::from_ref(&entry));
        let mut data: &[u8] = b"hello";
        let mut pairs: [(Entry, &mut dyn Read); 1] = [(entry, &mut data)];

        // ACT
        let mut buf = Vec::new();
        let written = ramune::archive::cpio(&mut pairs, &mut buf).expect("cpio");

        // ASSERT
        assert_eq!(written, expected);
        assert_eq!(u64::try_from(buf.len()).unwrap_or(0), expected);
    }
}
