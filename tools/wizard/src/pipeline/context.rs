//! Build inputs shared by preflight and the node runners.

use std::io::Write;
use std::sync::Mutex;

use sbolt::keys::SigningPair;

use crate::artifact::Artifact;
use crate::resolve::BuildPlan;

/// Build inputs, passed explicitly to preflight and runners.
///
/// The context is not part of the logical graph and never contains pipes.
pub(crate) struct BuildContext<'data, 'sign, 'writers> {
    pub(crate) plan: &'data BuildPlan,
    pub(crate) profile: &'data [u8],
    pub(crate) signing: Option<&'sign SigningPair<'sign>>,
    pub(crate) writers: Mutex<TargetWriters<'writers>>,
}

/// User artifact writers, consumed once each by their sink node.
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
