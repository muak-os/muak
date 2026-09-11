//! Build inputs shared by preflight and the node runners.

use std::io::Write;

use sbolt::keys::SigningPair;

use crate::artifact::Artifact;
use crate::domain::resolution::ResolvedBuild;

/// Build inputs, passed explicitly to planning, preflight, and runners.
pub(crate) struct BuildContext<'data, 'sign> {
    pub(crate) build: &'data ResolvedBuild,
    pub(crate) profile: &'data [u8],
    pub(crate) signing: Option<&'sign SigningPair<'sign>>,
}

/// User artifact writers, consumed once each by their producing node at bind time.
pub(crate) struct TargetWriters<'a> {
    slots: [Option<&'a mut (dyn Write + Send)>; Artifact::COUNT],
}

impl<'a> TargetWriters<'a> {
    /// Builds the slot array from the request's target pairs.
    #[must_use]
    pub(crate) fn new(targets: Vec<(Artifact, &'a mut (dyn Write + Send))>) -> Self {
        let mut slots: [Option<&'a mut (dyn Write + Send)>; Artifact::COUNT] =
            [const { None }; Artifact::COUNT];
        fill_slots(&mut slots, targets);

        Self { slots }
    }

    /// Takes the writer for an artifact, if still available.
    pub(crate) fn take(&mut self, artifact: Artifact) -> Option<&'a mut (dyn Write + Send)> {
        self.slots
            .get_mut(artifact.to_index())
            .and_then(Option::take)
    }
}

fn fill_slots<'a>(
    slots: &mut [Option<&'a mut (dyn Write + Send)>],
    targets: Vec<(Artifact, &'a mut (dyn Write + Send))>,
) {
    for (artifact, writer) in targets {
        if let Some(slot) = slots.get_mut(artifact.to_index()) {
            *slot = Some(writer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Sink;

    impl Write for Sink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn takes_each_writer_once() {
        // ARRANGE
        let mut first = Sink;
        let mut second = Sink;
        let mut writers = TargetWriters::new(vec![
            (Artifact::Kernel, &mut first),
            (Artifact::Iso, &mut second),
        ]);

        // ACT
        let kernel = writers.take(Artifact::Kernel);
        let kernel_again = writers.take(Artifact::Kernel);
        let iso = writers.take(Artifact::Iso);

        // ASSERT
        assert!(kernel.is_some(), "first take must yield the writer");
        assert!(kernel_again.is_none(), "a writer must be taken only once");
        assert!(iso.is_some(), "each artifact has its own slot");
    }
}
