//! Artifact sink node that touches user writers.

use std::io;

use crate::artifact::Artifact;
use crate::error::{Result, WizardError};
use crate::nodes::NodeKind;
use crate::nodes::{initramfs, installer, media, overlay, sign, uki};
use crate::pipeline::context::BuildContext;
use crate::pipeline::dependency::Dependency;
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::PortId;
use crate::pipeline::runtime::NodePorts;

pub(crate) const SINK_INPUT: PortId = PortId(0);

/// The requested artifact's stream.
pub(crate) fn dependencies(artifact: Artifact, ctx: &BuildContext<'_, '_, '_>) -> Vec<Dependency> {
    let (producer, producer_port) = artifact_source(artifact, ctx.signing.is_some());
    vec![Dependency::fixed(producer, producer_port, SINK_INPUT)]
}

/// Streams the artifact pipe into the user writer.
pub(crate) fn run(
    ctx: &BuildContext<'_, '_, '_>,
    artifact: Artifact,
    ports: &mut NodePorts<'_>,
) -> Result<NodeReport> {
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
        Artifact::Kernel => (NodeKind::InstallerPull, installer::KERNEL),
        Artifact::Cmdline => (NodeKind::InstallerPull, installer::CMDLINE),
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
    use crate::pipeline::context::TargetWriters;
    use crate::pipeline::runtime::{Endpoint, InputStream};
    use crate::request::Platform;
    use crate::resolve::BuildPlan;

    fn build_plan() -> BuildPlan {
        BuildPlan::new(
            Platform::Metal,
            "v1.0.0".to_owned(),
            Arch::Amd64,
            Vec::new(),
            None,
            "ghcr.io/muak-os/installer:v1.0.0".to_owned(),
        )
    }

    #[test]
    fn run_streams_input_into_the_artifact_writer() {
        // ARRANGE
        let (mut pipe_writer, pipe_reader) = UnixStream::pair().expect("pipe");
        pipe_writer
            .write_all(b"artifact bytes")
            .expect("write pipe");
        drop(pipe_writer);
        let plan = build_plan();
        let mut writer = Vec::new();
        let ctx = BuildContext {
            plan: &plan,
            profile: b"",
            signing: None,
            writers: Mutex::new(TargetWriters::new(vec![(Artifact::Kernel, &mut writer)])),
        };
        let mut ports = NodePorts {
            endpoints: vec![(
                SINK_INPUT,
                Endpoint::Input(InputStream {
                    size: 14,
                    name: "kernel",
                    reader: pipe_reader,
                }),
            )],
        };

        // ACT
        run(&ctx, Artifact::Kernel, &mut ports).expect("sink run");

        // ASSERT
        assert_eq!(writer, b"artifact bytes");
    }

    #[test]
    fn run_errors_when_the_writer_is_missing() {
        // ARRANGE
        let (pipe_writer, pipe_reader) = UnixStream::pair().expect("pipe");
        drop(pipe_writer);
        let plan = build_plan();
        let ctx = BuildContext {
            plan: &plan,
            profile: b"",
            signing: None,
            writers: Mutex::new(TargetWriters::new(Vec::new())),
        };
        let mut ports = NodePorts {
            endpoints: vec![(
                SINK_INPUT,
                Endpoint::Input(InputStream {
                    size: 0,
                    name: "kernel",
                    reader: pipe_reader,
                }),
            )],
        };

        // ACT
        let error = run(&ctx, Artifact::Kernel, &mut ports)
            .err()
            .expect("missing writer");

        // ASSERT
        let message = error.to_string();
        assert!(
            message.contains("missing target writer for kernel"),
            "unexpected error: {message}"
        );
    }
}
