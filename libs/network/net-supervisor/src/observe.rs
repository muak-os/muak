//! Journal adapters for the supervisor vocabulary.

use net_observe::journal::Journal;
use net_observe::{Event, Level};

use crate::supervisor::StatusKey;
use crate::{EffectKind, State};

impl From<State> for net_observe::SupervisorState {
    fn from(state: State) -> Self {
        match state {
            State::Uninitialized => Self::Uninitialized,
            State::Initializing => Self::Initializing,
            State::Operational => Self::Operational,
            State::Ready => Self::Ready,
            State::Degraded => Self::Degraded,
        }
    }
}

impl From<EffectKind> for net_observe::EffectKind {
    fn from(kind: EffectKind) -> Self {
        match kind {
            EffectKind::Discover => Self::Discover,
            EffectKind::ProbeCarriers => Self::ProbeCarriers,
            EffectKind::SetLinkUp => Self::SetLinkUp,
            EffectKind::ConfigureInterface => Self::ConfigureInterface,
            EffectKind::ReconcileInterface => Self::ReconcileInterface,
            EffectKind::EnsureBridge => Self::EnsureBridge,
            EffectKind::Enslave => Self::Enslave,
            EffectKind::Detach => Self::Detach,
            EffectKind::ShutdownInterface => Self::ShutdownInterface,
            EffectKind::Publish => Self::Publish,
            EffectKind::WriteResolver => Self::WriteResolver,
        }
    }
}

/// Records the status differences a dispatch produced.
pub(crate) fn record(journal: &mut Journal, before: &StatusKey, after: &StatusKey) {
    if before.1 != after.1 || before.4 != after.4 {
        journal.record(
            Level::Info,
            Event::PrimaryChanged {
                primary: after.1.clone(),
                backups: after.4.clone(),
            },
        );
    }

    let transient = after.2.is_some() && after.3.is_none();
    if !transient && (before.2 != after.2 || before.3 != after.3) {
        journal.record(
            Level::Info,
            Event::OwnerChanged {
                owner: after.2.clone(),
                link: after.3,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use net_observe::SupervisorState;
    use net_observe::journal::Journal;

    use crate::{EffectKind, State, observe};

    #[test]
    fn state_conversion_covers_every_variant() {
        // ARRANGE
        let cases = [
            (State::Uninitialized, SupervisorState::Uninitialized),
            (State::Initializing, SupervisorState::Initializing),
            (State::Operational, SupervisorState::Operational),
            (State::Ready, SupervisorState::Ready),
            (State::Degraded, SupervisorState::Degraded),
        ];

        // ACT / ASSERT
        for (source, expected) in cases {
            assert_eq!(SupervisorState::from(source), expected);
        }
    }

    #[test]
    fn effect_conversion_covers_every_variant() {
        // ARRANGE
        let cases = [
            (EffectKind::Discover, net_observe::EffectKind::Discover),
            (
                EffectKind::ProbeCarriers,
                net_observe::EffectKind::ProbeCarriers,
            ),
            (EffectKind::SetLinkUp, net_observe::EffectKind::SetLinkUp),
            (
                EffectKind::ConfigureInterface,
                net_observe::EffectKind::ConfigureInterface,
            ),
            (
                EffectKind::ReconcileInterface,
                net_observe::EffectKind::ReconcileInterface,
            ),
            (
                EffectKind::EnsureBridge,
                net_observe::EffectKind::EnsureBridge,
            ),
            (EffectKind::Enslave, net_observe::EffectKind::Enslave),
            (EffectKind::Detach, net_observe::EffectKind::Detach),
            (
                EffectKind::ShutdownInterface,
                net_observe::EffectKind::ShutdownInterface,
            ),
            (EffectKind::Publish, net_observe::EffectKind::Publish),
            (
                EffectKind::WriteResolver,
                net_observe::EffectKind::WriteResolver,
            ),
        ];

        // ACT / ASSERT
        for (source, expected) in cases {
            assert_eq!(net_observe::EffectKind::from(source), expected);
        }
    }

    #[test]
    fn unchanged_status_records_nothing() {
        // ARRANGE
        let key = (State::Ready, None, None, None, Vec::new());
        let mut journal = Journal::new();

        // ACT
        observe::record(&mut journal, &key, &key);

        // ASSERT
        assert!(journal.is_empty());
    }
}
