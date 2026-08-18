//! The initramfs CPIO tail builder.

use std::io::Read;

use ramune::Entry;

use crate::error::{Result, WizardError};
use crate::nodes::extensions;
use crate::nodes::{NodeDescriptor, NodeKind, no_dynamic_output_count};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::{Graph, NodeId, PortId};
use crate::pipeline::runtime::{Endpoint, NodePorts};

pub(crate) const TAIL_OUTPUT: PortId = PortId(0);
pub(crate) const TAIL_INPUTS_FIRST: PortId = PortId(1);

pub(crate) const DESCRIPTOR: NodeDescriptor = NodeDescriptor {
    dependencies,
    output_count: no_dynamic_output_count,
    preflight,
    run,
};

/// One extension payload stream per extension, in canonical source order.
fn dependencies(_kind: NodeKind, _ctx: &BuildContext<'_, '_, '_>) -> Vec<Dependency> {
    vec![Dependency::many(
        NodeKind::ExtensionPayloads,
        extensions::FIRST_OUTPUT,
        TAIL_INPUTS_FIRST,
    )]
}

/// Exact CPIO tail size from the named extension input streams plus the profile entry.
fn preflight(graph: &mut Graph, id: NodeId, ctx: &BuildContext<'_, '_, '_>) -> Result<()> {
    let mut entries =
        Vec::with_capacity(graph.node(id)?.input_bindings().count().saturating_add(1));
    for binding in graph.node(id)?.input_bindings() {
        let stream = graph.stream(binding.stream)?;
        entries.push(Entry {
            path: stream.name.clone(),
            mode: 0o100_644,
            len: stream.size,
        });
    }
    let profile_len = u64::try_from(ctx.profile.len())
        .map_err(|e| WizardError::BuildError(format!("profile size overflow: {e}")))?;
    if !ctx.profile.is_empty() {
        entries.push(profile_entry(profile_len));
    }
    let tail = ramune::archive::size(&entries);

    let bindings = graph
        .node(id)?
        .output_bindings()
        .copied()
        .collect::<Vec<_>>();
    for binding in bindings {
        let stream = graph.stream_mut(binding.stream)?;
        stream.size = tail;
        "tail.cpio".clone_into(&mut stream.name);
    }

    Ok(())
}

/// Streams one CPIO entry per extension input stream plus the profile entry in canonical order.
fn run(
    _kind: NodeKind,
    ports: &mut NodePorts<'_>,
    ctx: &BuildContext<'_, '_, '_>,
) -> Result<NodeReport> {
    let mut inputs = Endpoint::into_inputs(
        ports
            .take_from(TAIL_INPUTS_FIRST, None)?
            .into_iter()
            .map(|(_, endpoint)| endpoint),
    )?;

    let mut pairs: Vec<(Entry, &mut dyn Read)> = Vec::with_capacity(inputs.len().saturating_add(1));
    for input in &mut inputs {
        let reader: &mut dyn Read = &mut input.reader;
        pairs.push((
            Entry {
                path: input.name.to_owned(),
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
    ramune::archive::cpio(&mut pairs, &mut output.writer)
        .map_err(|e| WizardError::BuildError(format!("build initramfs tail: {e}")))?;

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
