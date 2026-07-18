use std::io::Write;

use crate::artifact::Artifact;

/// Maps `Artifact` kinds to optional output writers.
pub(crate) struct Router<'a> {
    slots: [Option<&'a mut dyn Write>; Artifact::COUNT],
}

impl<'a> Router<'a> {
    pub(crate) fn new(targets: Vec<(Artifact, &'a mut dyn Write)>) -> Self {
        let mut slots: [Option<&'a mut dyn Write>; Artifact::COUNT] =
            [const { None }; Artifact::COUNT];
        for (kind, writer) in targets {
            *slots
                .get_mut(kind.to_index())
                .map_or(&mut None, |slot| slot) = Some(writer);
        }

        Self { slots }
    }

    pub(crate) fn take(&mut self, kind: Artifact) -> Option<&'a mut dyn Write> {
        self.slots.get_mut(kind.to_index()).and_then(Option::take)
    }
}
