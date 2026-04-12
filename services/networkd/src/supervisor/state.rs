//! Global network readiness state machine for the supervisor.

use crate::state_machine::StateMachine;

/// Status of the entire network subsystem.
#[derive(Debug, Clone, PartialEq)]
pub enum NetworkState {
    Uninitialized,
    Initializing,
    Operational,
    Ready,
    Degraded,
}

impl StateMachine for NetworkState {
    fn valid_next_states(&self) -> &'static [Self] {
        match self {
            Self::Uninitialized => &[Self::Initializing],
            Self::Initializing => &[Self::Operational, Self::Degraded],
            Self::Operational => &[Self::Ready, Self::Degraded],
            Self::Ready => &[Self::Degraded],
            Self::Degraded => &[Self::Initializing, Self::Operational],
        }
    }
}

impl std::fmt::Display for NetworkState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Uninitialized => "Uninitialized",
            Self::Initializing => "Initializing",
            Self::Operational => "Operational",
            Self::Ready => "Ready",
            Self::Degraded => "Degraded",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_snap() -> crate::supervisor::snapshot::NetworkSnapshot {
        crate::supervisor::snapshot::NetworkSnapshot::empty()
    }

    #[test]
    fn network_state_display() {
        // ACT / ASSERT
        assert_eq!(NetworkState::Uninitialized.to_string(), "Uninitialized");
        assert_eq!(NetworkState::Initializing.to_string(), "Initializing");
        assert_eq!(NetworkState::Operational.to_string(), "Operational");
        assert_eq!(NetworkState::Ready.to_string(), "Ready");
        assert_eq!(NetworkState::Degraded.to_string(), "Degraded");
    }

    #[test]
    fn valid_transitions_succeed() {
        let pairs = [
            (NetworkState::Uninitialized, NetworkState::Initializing),
            (NetworkState::Initializing, NetworkState::Operational),
            (NetworkState::Initializing, NetworkState::Degraded),
            (NetworkState::Operational, NetworkState::Ready),
            (NetworkState::Operational, NetworkState::Degraded),
            (NetworkState::Ready, NetworkState::Degraded),
            (NetworkState::Degraded, NetworkState::Initializing),
            (NetworkState::Degraded, NetworkState::Operational),
        ];

        for (from, to) in pairs {
            // ARRANGE
            let mut snap = empty_snap();
            snap.state = from.clone();

            // ACT
            let result = snap.transition(to.clone());

            // ASSERT
            assert!(result.is_ok(), "{from} -> {to} should be valid");
            assert_eq!(snap.state, to);
        }
    }

    #[test]
    fn invalid_transitions_return_error() {
        let pairs = [
            (NetworkState::Uninitialized, NetworkState::Operational),
            (NetworkState::Uninitialized, NetworkState::Ready),
            (NetworkState::Uninitialized, NetworkState::Degraded),
            (NetworkState::Initializing, NetworkState::Ready),
            (NetworkState::Initializing, NetworkState::Uninitialized),
            (NetworkState::Operational, NetworkState::Uninitialized),
            (NetworkState::Operational, NetworkState::Initializing),
            (NetworkState::Ready, NetworkState::Operational),
            (NetworkState::Ready, NetworkState::Uninitialized),
            (NetworkState::Ready, NetworkState::Initializing),
            (NetworkState::Degraded, NetworkState::Ready),
            (NetworkState::Degraded, NetworkState::Uninitialized),
        ];

        for (from, to) in pairs {
            // ARRANGE
            let mut snap = empty_snap();
            snap.state = from.clone();

            // ACT
            let result = snap.transition(to.clone());

            // ASSERT
            assert!(result.is_err(), "{from} -> {to} should be invalid");
            assert_eq!(
                snap.state, from,
                "state must not change on invalid transition"
            );
        }
    }

    #[test]
    fn transition_does_not_mutate_state_on_error() {
        // ARRANGE
        let mut snap = empty_snap();
        snap.state = NetworkState::Ready;

        // ACT
        let _ = snap.transition(NetworkState::Initializing);

        // ASSERT
        assert_eq!(snap.state, NetworkState::Ready);
    }
}
