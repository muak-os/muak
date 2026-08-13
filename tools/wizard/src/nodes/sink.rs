//! Artifact sink node that holds a user writer.

use std::io;

use crate::error::{Result, WizardError};
use crate::pipeline::execute::NodeReport;
use crate::pipeline::graph::PortId;
use crate::pipeline::runtime::PreparedSink;

pub(crate) const SINK_INPUT: PortId = PortId(0);

/// Streams the artifact pipe into the user writer.
pub(crate) fn run(sink: PreparedSink<'_>) -> Result<NodeReport> {
    let mut input = sink.input;
    io::copy(&mut input.reader, sink.writer)
        .map_err(|e| WizardError::BuildError(format!("sink stream: {e}")))?;

    Ok(NodeReport::Empty)
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::os::unix::net::UnixStream;

    use super::*;
    use crate::pipeline::runtime::InputStream;

    #[test]
    fn run_streams_input_into_the_writer() {
        // ARRANGE
        let (mut pipe_writer, pipe_reader) = UnixStream::pair().expect("pipe");
        pipe_writer
            .write_all(b"artifact bytes")
            .expect("write pipe");
        drop(pipe_writer);
        let mut writer = Vec::new();
        let sink = PreparedSink {
            input: InputStream {
                size: 14,
                reader: pipe_reader,
            },
            writer: &mut writer,
        };

        // ACT
        run(sink).expect("sink run");

        // ASSERT
        assert_eq!(writer, b"artifact bytes");
    }
}
