//! Artifact sink node that touches user writers.

use std::io;

use crate::artifact::Artifact;
use crate::error::{Result, WizardError};
use crate::nodes::NodeKind;
use crate::nodes::{NodeDescriptor, no_dynamic_output_count};
use crate::nodes::{initramfs, kernel, media, overlay, sign, uki};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::{Graph, NodeId, PortId};
use crate::pipeline::runtime::NodePorts;

pub(crate) const SINK_INPUT: PortId = PortId(0);

pub(crate) const DESCRIPTOR: NodeDescriptor = NodeDescriptor {
    dependencies,
    output_count: no_dynamic_output_count,
    preflight,
    run,
};

/// The requested artifact's stream.
fn dependencies(kind: NodeKind, ctx: &BuildContext<'_, '_, '_>) -> Vec<Dependency> {
    let NodeKind::ArtifactSink { artifact } = kind else {
        return Vec::new();
    };
    let (producer, producer_port) = artifact_source(artifact, ctx.signing.is_some());
    vec![Dependency::fixed(producer, producer_port, SINK_INPUT)]
}

/// Confirms the sink's input dependency is bound to a stream.
fn preflight(graph: &mut Graph, id: NodeId, _ctx: &BuildContext<'_, '_, '_>) -> Result<()> {
    graph.node(id)?.input(SINK_INPUT).map(|_| ())
}

/// Streams the artifact pipe into the user writer.
fn run(
    kind: NodeKind,
    ports: &mut NodePorts<'_>,
    ctx: &BuildContext<'_, '_, '_>,
) -> Result<NodeReport> {
    let NodeKind::ArtifactSink { artifact } = kind else {
        return Err(WizardError::BuildError(
            "sink run dispatched for a non-sink kind".to_owned(),
        ));
    };
    let input = ports.take(SINK_INPUT)?.into_input()?;
    let mut writers = ctx
        .writers
        .lock()
        .map_err(|_poisoned| WizardError::BuildError("target writers mutex poisoned".to_owned()))?;
    let writer = writers
        .take(artifact)
        .ok_or_else(|| WizardError::BuildError(format!("missing target writer for {artifact}")))?;
    drop(writers);

    let mut input = input;
    io::copy(&mut input.reader, writer)
        .map_err(|e| WizardError::BuildError(format!("sink stream: {e}")))?;

    Ok(NodeReport::Empty)
}

/// The producer of each requested artifact's stream.
fn artifact_source(artifact: Artifact, signed: bool) -> (NodeKind, PortId) {
    match artifact {
        Artifact::Kernel => (NodeKind::KernelPull, kernel::KERNEL),
        Artifact::Cmdline => (NodeKind::KernelPull, kernel::CMDLINE),
        Artifact::Initramfs => (NodeKind::Concat, initramfs::concat::CONCAT_OUTPUT),
        Artifact::Uki if signed => (NodeKind::Sign, sign::SIGN_OUTPUT),
        Artifact::Uki => (NodeKind::Uki, uki::UKI_OUTPUT),
        Artifact::Iso => (NodeKind::Iso, media::MEDIA_OUTPUT),
        Artifact::Raw => (NodeKind::Raw, media::MEDIA_OUTPUT),
        Artifact::Overlays => (NodeKind::OverlayTar, overlay::tar::TAR_OUTPUT),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::os::unix::net::UnixStream;
    use std::sync::Mutex;

    use koci::arch::Arch;

    use super::*;
    use crate::domain::resolution::Kernel;
    use crate::domain::resolution::ResolvedBuild;
    use crate::pipeline::context::TargetWriters;
    use crate::pipeline::runtime::{Endpoint, InputStream};
    use crate::request::Platform;

    fn build_plan() -> ResolvedBuild {
        ResolvedBuild::new(
            Platform::Metal,
            "v1.0.0".to_owned(),
            Arch::Amd64,
            Vec::new(),
            None,
            Kernel::new(
                "ghcr.io/muak-os/kernel".to_owned(),
                "ghcr.io/muak-os/kernel:v1.0.0".to_owned(),
            ),
            "ghcr.io/muak-os/installer:v1.0.0".to_owned(),
        )
    }

    fn context(build: &ResolvedBuild) -> BuildContext<'_, '_, '_> {
        BuildContext {
            build,
            profile: b"",
            signing: None,
            writers: Mutex::new(TargetWriters::new(Vec::new())),
        }
    }

    fn sink_input(reader: UnixStream) -> NodePorts<'static> {
        NodePorts {
            endpoints: vec![(
                SINK_INPUT,
                Endpoint::Input(InputStream {
                    size: 0,
                    name: "kernel",
                    reader,
                }),
            )],
        }
    }

    #[test]
    fn run_streams_input_into_the_artifact_writer() {
        // ARRANGE
        let (mut pipe_writer, pipe_reader) = UnixStream::pair().expect("pipe");
        pipe_writer
            .write_all(b"artifact bytes")
            .expect("write pipe");
        drop(pipe_writer);
        let build = build_plan();
        let mut writer = Vec::new();
        let ctx = BuildContext {
            build: &build,
            profile: b"",
            signing: None,
            writers: Mutex::new(TargetWriters::new(vec![(Artifact::Kernel, &mut writer)])),
        };
        let mut ports = sink_input(pipe_reader);

        // ACT
        run(
            NodeKind::ArtifactSink {
                artifact: Artifact::Kernel,
            },
            &mut ports,
            &ctx,
        )
        .expect("sink run");

        // ASSERT
        assert_eq!(writer, b"artifact bytes");
    }

    #[test]
    fn run_errors_when_the_writer_is_missing() {
        // ARRANGE
        let (pipe_writer, pipe_reader) = UnixStream::pair().expect("pipe");
        drop(pipe_writer);
        let build = build_plan();
        let ctx = context(&build);
        let mut ports = sink_input(pipe_reader);

        // ACT
        let error = run(
            NodeKind::ArtifactSink {
                artifact: Artifact::Kernel,
            },
            &mut ports,
            &ctx,
        )
        .err()
        .expect("missing writer");

        // ASSERT
        let message = error.to_string();
        assert!(
            message.contains("missing target writer for kernel"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn run_rejects_a_non_sink_kind() {
        // ARRANGE
        let build = build_plan();
        let ctx = context(&build);
        let mut ports = NodePorts {
            endpoints: Vec::new(),
        };

        // ACT
        let error = run(NodeKind::Concat, &mut ports, &ctx)
            .err()
            .expect("non-sink kind");

        // ASSERT
        let message = error.to_string();
        assert!(
            message.contains("non-sink kind"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn preflight_confirms_the_input_binding() {
        // ARRANGE
        let build = build_plan();
        let ctx = context(&build);
        let mut graph = Graph::new();
        let producer = graph.add_node(NodeKind::KernelPull);
        let sink = graph.add_node(NodeKind::ArtifactSink {
            artifact: Artifact::Kernel,
        });
        let stream = graph.add_output(producer, PortId(0)).expect("add output");
        graph
            .bind_input(sink, SINK_INPUT, stream)
            .expect("bind input");

        // ACT
        let result = preflight(&mut graph, sink, &ctx);

        // ASSERT
        result.expect("sink preflight");
    }

    #[test]
    fn preflight_rejects_an_unbound_sink() {
        // ARRANGE
        let build = build_plan();
        let ctx = context(&build);
        let mut graph = Graph::new();
        let sink = graph.add_node(NodeKind::ArtifactSink {
            artifact: Artifact::Kernel,
        });

        // ACT
        let result = preflight(&mut graph, sink, &ctx);

        // ASSERT
        result.unwrap_err();
    }
}
