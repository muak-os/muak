//! Builds the initial logical graph for a request.

use crate::artifact::Artifact;
use crate::error::{Result, WizardError};
use crate::nodes::{extensions, initramfs, installer, media, overlays, uki};
use crate::pipeline::graph::{Graph, NodeId, NodeKind, PortId, StreamId};
use crate::pipeline::normalize::normalize;
use crate::resolve::BuildPlan;

/// Builds the logical DAG for the requested artifacts and normalizes it.
///
/// # Errors
///
/// Returns an error when the overlay file listing cannot be fetched.
pub(crate) async fn plan(build: &BuildPlan, artifacts: &[Artifact]) -> Result<Graph> {
    let wants: Vec<Artifact> = artifacts.to_vec();
    let is_wanted = |artifact: Artifact| wants.contains(&artifact);

    let needs_initramfs = is_wanted(Artifact::Initramfs)
        || is_wanted(Artifact::Uki)
        || is_wanted(Artifact::Iso)
        || is_wanted(Artifact::Raw);
    let needs_uki =
        is_wanted(Artifact::Uki) || is_wanted(Artifact::Iso) || is_wanted(Artifact::Raw);
    let needs_media = is_wanted(Artifact::Iso) || is_wanted(Artifact::Raw);
    let needs_overlays = build.overlay().is_some() && is_wanted(Artifact::Overlays);

    let mut graph = Graph::new();
    let installer_node = graph.add_node(NodeKind::InstallerPull);

    let mut stub = None;
    let mut cmdline = None;
    let mut kernel = None;

    if needs_uki {
        stub = Some(graph.add_output(installer_node, installer::STUB)?);
    }
    if is_wanted(Artifact::Cmdline) || needs_uki {
        let stream = graph.add_output(installer_node, installer::CMDLINE)?;
        if is_wanted(Artifact::Cmdline) {
            let cmdline_sink = sink(&mut graph, Artifact::Cmdline);
            graph.bind_input(cmdline_sink, PortId(0), stream)?;
        }
        cmdline = Some(stream);
    }
    if is_wanted(Artifact::Kernel) || needs_uki {
        let stream = graph.add_output(installer_node, installer::KERNEL)?;
        if is_wanted(Artifact::Kernel) {
            let kernel_sink = sink(&mut graph, Artifact::Kernel);
            graph.bind_input(kernel_sink, PortId(0), stream)?;
        }
        kernel = Some(stream);
    }

    let complete = if needs_initramfs {
        add_initramfs_chain(build, &mut graph, installer_node, is_wanted)?
    } else {
        None
    };

    let uki_output = if needs_uki {
        add_uki(
            &mut graph,
            required(stub, "stub stream")?,
            required(cmdline, "cmdline stream")?,
            required(kernel, "kernel stream")?,
            required(complete, "initramfs stream")?,
            is_wanted,
        )?
    } else {
        None
    };

    let iso = is_wanted(Artifact::Iso).then(|| graph.add_node(NodeKind::Iso));
    let raw = is_wanted(Artifact::Raw).then(|| graph.add_node(NodeKind::Raw));
    for media_node in [iso, raw].into_iter().flatten() {
        graph.bind_input(
            media_node,
            media::MEDIA_UKI,
            required(uki_output, "uki stream")?,
        )?;
    }

    if let Some(overlay) = build.overlay()
        && (needs_media || needs_overlays)
    {
        let files = overlays::listing(overlay).await?;
        add_overlay_nodes(&mut graph, &files, iso, raw, needs_overlays)?;
    }

    normalize(&mut graph)?;

    Ok(graph)
}

/// Wires overlay file streams to media and tar consumers.
fn add_overlay_nodes(
    graph: &mut Graph,
    files: &[(String, u64)],
    iso: Option<NodeId>,
    raw: Option<NodeId>,
    wants_tar: bool,
) -> Result<()> {
    let pull_node = graph.add_node(NodeKind::OverlayPull);
    let tar = wants_tar.then(|| graph.add_node(NodeKind::OverlayTar));
    for (index, _) in files.iter().enumerate() {
        let stream = graph.add_output(pull_node, dyn_port(overlays::PULL_OUTPUTS_FIRST, index))?;
        if let Some(tar_node) = tar {
            graph.bind_input(
                tar_node,
                dyn_port(overlays::TAR_INPUTS_FIRST, index),
                stream,
            )?;
        }
        for media_node in [iso, raw].into_iter().flatten() {
            graph.bind_input(
                media_node,
                dyn_port(media::MEDIA_OVERLAYS_FIRST, index),
                stream,
            )?;
        }
    }
    if let Some(tar_node) = tar {
        let output = graph.add_output(tar_node, overlays::TAR_OUTPUT)?;
        let overlays_sink = sink(graph, Artifact::Overlays);
        graph.bind_input(overlays_sink, PortId(0), output)?;
    }

    Ok(())
}

/// Wires the installer base initramfs, the extension payloads, the tail,
/// and the Concat node into the complete initramfs stream.
fn add_initramfs_chain(
    build: &BuildPlan,
    graph: &mut Graph,
    installer_node: NodeId,
    is_wanted: impl Fn(Artifact) -> bool,
) -> Result<Option<StreamId>> {
    let base = graph.add_output(installer_node, installer::INITRAMFS)?;
    let extensions_node = graph.add_node(NodeKind::ExtensionPayloads);
    let tail = graph.add_node(NodeKind::InitramfsTail);
    for (index, _) in build.extensions().iter().enumerate() {
        let payload =
            graph.add_output(extensions_node, dyn_port(extensions::FIRST_OUTPUT, index))?;
        graph.bind_input(tail, dyn_port(initramfs::TAIL_INPUTS_FIRST, index), payload)?;
    }
    let concat = graph.add_node(NodeKind::Concat);
    graph.bind_input(concat, initramfs::CONCAT_BASE, base)?;
    let tail_stream = graph.add_output(tail, initramfs::TAIL_OUTPUT)?;
    graph.bind_input(concat, initramfs::CONCAT_TAIL, tail_stream)?;
    let complete = graph.add_output(concat, initramfs::CONCAT_OUTPUT)?;
    if is_wanted(Artifact::Initramfs) {
        let initramfs_sink = sink(graph, Artifact::Initramfs);
        graph.bind_input(initramfs_sink, PortId(0), complete)?;
    }

    Ok(Some(complete))
}

/// Wires the UKI node and returns its output stream.
fn add_uki(
    graph: &mut Graph,
    stub: StreamId,
    cmdline: StreamId,
    kernel: StreamId,
    complete: StreamId,
    is_wanted: impl Fn(Artifact) -> bool,
) -> Result<Option<StreamId>> {
    let uki_node = graph.add_node(NodeKind::Uki);
    graph.bind_input(uki_node, uki::UKI_STUB, stub)?;
    graph.bind_input(uki_node, uki::UKI_CMDLINE, cmdline)?;
    graph.bind_input(uki_node, uki::UKI_KERNEL, kernel)?;
    graph.bind_input(uki_node, uki::UKI_INITRAMFS, complete)?;
    let output = graph.add_output(uki_node, uki::UKI_OUTPUT)?;
    if is_wanted(Artifact::Uki) {
        let uki_sink = sink(graph, Artifact::Uki);
        graph.bind_input(uki_sink, PortId(0), output)?;
    }

    Ok(Some(output))
}

fn dyn_port(first: PortId, index: usize) -> PortId {
    PortId(first.0.saturating_add(index))
}

fn required(stream: Option<StreamId>, what: &str) -> Result<StreamId> {
    stream.ok_or_else(|| WizardError::BuildError(format!("planner: missing {what}")))
}

fn sink(graph: &mut Graph, artifact: Artifact) -> NodeId {
    graph.add_node(NodeKind::ArtifactSink { artifact })
}

#[cfg(test)]
mod tests {
    use koci::arch::Arch;

    use super::*;
    use crate::request::Platform;
    use crate::source::extension::Extension;

    fn build_plan() -> BuildPlan {
        // ARRANGE
        BuildPlan::new(
            Platform::Metal,
            "v1.0.0".to_owned(),
            Arch::Amd64,
            vec![Extension::new(
                "muak-os/qemu".to_owned(),
                "ghcr.io/muak-os/qemu:v1.0.0".to_owned(),
            )],
            None,
            "ghcr.io/muak-os/installer:v1.0.0".to_owned(),
        )
    }

    fn kinds(graph: &Graph) -> Vec<NodeKind> {
        graph.nodes().iter().map(|node| node.kind.clone()).collect()
    }

    fn count(graph: &Graph, kind: &NodeKind) -> usize {
        graph
            .nodes()
            .iter()
            .filter(|node| &node.kind == kind)
            .count()
    }

    #[tokio::test]
    async fn kernel_and_cmdline_need_only_installer() {
        // ARRANGE
        let build = build_plan();

        // ACT
        let graph = plan(&build, &[Artifact::Kernel, Artifact::Cmdline])
            .await
            .expect("plan");

        // ASSERT
        assert_eq!(kinds(&graph).len(), 3);
        assert_eq!(count(&graph, &NodeKind::InstallerPull), 1);
        assert_eq!(
            count(
                &graph,
                &NodeKind::ArtifactSink {
                    artifact: Artifact::Kernel
                }
            ),
            1
        );
        assert_eq!(
            count(
                &graph,
                &NodeKind::ArtifactSink {
                    artifact: Artifact::Cmdline
                }
            ),
            1
        );
        assert_eq!(count(&graph, &NodeKind::Fanout), 0);
    }

    #[tokio::test]
    async fn initramfs_and_uki_fanout_the_complete_initramfs() {
        // ARRANGE
        let build = build_plan();

        // ACT
        let graph = plan(&build, &[Artifact::Initramfs, Artifact::Uki])
            .await
            .expect("plan");

        // ASSERT
        assert_eq!(count(&graph, &NodeKind::Uki), 1);
        assert_eq!(count(&graph, &NodeKind::Concat), 1);
        assert_eq!(count(&graph, &NodeKind::InitramfsTail), 1);
        assert_eq!(count(&graph, &NodeKind::ExtensionPayloads), 1);
        assert_eq!(count(&graph, &NodeKind::Fanout), 1);
        assert_eq!(
            count(
                &graph,
                &NodeKind::ArtifactSink {
                    artifact: Artifact::Initramfs
                }
            ),
            1
        );
        assert_eq!(
            count(
                &graph,
                &NodeKind::ArtifactSink {
                    artifact: Artifact::Uki
                }
            ),
            1
        );
    }

    #[tokio::test]
    async fn uki_iso_and_raw_fanout_the_uki_stream() {
        // ARRANGE
        let build = build_plan();

        // ACT
        let graph = plan(&build, &[Artifact::Uki, Artifact::Iso, Artifact::Raw])
            .await
            .expect("plan");

        // ASSERT
        assert_eq!(count(&graph, &NodeKind::Uki), 1);
        assert_eq!(count(&graph, &NodeKind::Iso), 1);
        assert_eq!(count(&graph, &NodeKind::Raw), 1);
        assert_eq!(count(&graph, &NodeKind::Fanout), 1);
        assert_eq!(count(&graph, &NodeKind::OverlayPull), 0);
    }

    #[test]
    fn iso_and_overlays_wire_overlay_once_through_fanout() {
        // ARRANGE
        let mut graph = Graph::new();
        let installer_node = graph.add_node(NodeKind::InstallerPull);
        graph
            .add_output(installer_node, installer::STUB)
            .expect("add output");
        let iso = graph.add_node(NodeKind::Iso);
        let raw = graph.add_node(NodeKind::Raw);
        let files = vec![("a.txt".to_owned(), 3), ("b.bin".to_owned(), 5)];

        // ACT
        add_overlay_nodes(&mut graph, &files, Some(iso), Some(raw), true).expect("add overlay");
        normalize(&mut graph).expect("normalize");

        // ASSERT
        assert_eq!(count(&graph, &NodeKind::OverlayPull), 1);
        assert_eq!(count(&graph, &NodeKind::OverlayTar), 1);
        assert_eq!(count(&graph, &NodeKind::Fanout), 2);
        for node in graph
            .nodes()
            .iter()
            .filter(|node| matches!(&node.kind, NodeKind::Fanout))
        {
            assert_eq!(node.outputs.len(), 3);
        }
    }

    #[tokio::test]
    async fn binds_stable_uki_ports() {
        // ARRANGE
        let build = build_plan();

        // ACT
        let graph = plan(&build, &[Artifact::Uki]).await.expect("plan");

        // ACT
        let uki_node = graph
            .nodes()
            .iter()
            .find(|node| matches!(&node.kind, NodeKind::Uki))
            .expect("uki node");

        // ASSERT
        assert!(uki_node.input(uki::UKI_STUB).is_ok(), "missing stub input");
        assert!(
            uki_node.input(uki::UKI_CMDLINE).is_ok(),
            "missing cmdline input"
        );
        assert!(
            uki_node.input(uki::UKI_KERNEL).is_ok(),
            "missing kernel input"
        );
        assert!(
            uki_node.input(uki::UKI_INITRAMFS).is_ok(),
            "missing initramfs input"
        );
        uki_node.output(uki::UKI_OUTPUT).unwrap();
    }
}
