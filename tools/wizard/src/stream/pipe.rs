//! Runtime pipe abstraction.

use std::os::unix::net::UnixStream;

use crate::error::{Result, WizardError};

/// A bounded byte pipe between one producer and one consumer.
pub(crate) struct Pipe {
    reader: UnixStream,
    writer: UnixStream,
}

impl Pipe {
    /// Creates a new pipe pair.
    pub(crate) fn new(context: &'static str) -> Result<Self> {
        let (reader, writer) = UnixStream::pair()
            .map_err(|source| WizardError::BuildError(format!("{context}: {source}")))?;

        Ok(Self { reader, writer })
    }

    /// Splits the pipe into its reader and writer ends.
    pub(crate) fn split(self) -> (UnixStream, UnixStream) {
        (self.reader, self.writer)
    }
}
