use crate::artifact::Artifact;
use std::io::Write;

/// Maps `Artifact` kinds to optional output writers. Indexed by discriminant.
pub(crate) struct Router<'a> {
    slots: [Option<&'a mut dyn Write>; Artifact::COUNT],
}

fn assign_slot<'a>(
    slots: &mut [Option<&'a mut dyn Write>; Artifact::COUNT],
    kind: Artifact,
    writer: &'a mut dyn Write,
) {
    let idx = kind.discriminant();
    let Some(slot) = slots.get_mut(idx) else {
        return;
    };

    *slot = Some(writer);
}

impl<'a> Router<'a> {
    pub(crate) fn new(targets: Vec<(Artifact, &'a mut dyn Write)>) -> Self {
        let mut slots: [Option<&'a mut dyn Write>; Artifact::COUNT] =
            [const { None }; Artifact::COUNT];
        for (kind, writer) in targets {
            assign_slot(&mut slots, kind, writer);
        }

        Self { slots }
    }

    pub(crate) fn take(&mut self, kind: Artifact) -> Option<&'a mut dyn Write> {
        self.slots
            .get_mut(kind.discriminant())
            .and_then(Option::take)
    }
}
