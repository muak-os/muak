//! Internal network state with validated transitions.

use netmodel::Name;

use crate::State;

/// Readiness plus the elected primary, backups, and layer 3 owner.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Snapshot {
    /// Global readiness.
    pub(crate) state: State,
    /// Elected uplink that `auto` resolves to.
    pub(crate) primary: Option<Name>,
    /// Interface that owns layer 3.
    pub(crate) l3_owner: Option<Name>,
    /// Uplink candidates in failover preference order.
    pub(crate) backups: Vec<Name>,
    /// State transitions recorded since the last drain, in order.
    ///
    /// The queue keeps intermediate transitions visible even when one step
    /// changes the state more than once.
    pub(crate) transitions: Vec<(State, State)>,
}

impl Snapshot {
    /// Returns true when the network is operational or ready.
    pub(crate) const fn is_operational(&self) -> bool {
        matches!(self.state, State::Operational | State::Ready)
    }

    /// Advances to `to` when the transition is valid, leaving state untouched otherwise.
    pub(crate) fn transition(&mut self, to: State) -> bool {
        if valid(self.state, to) {
            self.transitions.push((self.state, to));
            self.state = to;
            return true;
        }

        false
    }

    /// Advances to `Operational` unless the network is already operational or ready.
    pub(crate) fn ensure_operational(&mut self) -> bool {
        if self.is_operational() {
            return true;
        }

        self.transition(State::Operational)
    }

    /// Advances to `Degraded` unless the network is already degraded.
    pub(crate) fn ensure_degraded(&mut self) -> bool {
        if matches!(self.state, State::Degraded) {
            return true;
        }

        self.transition(State::Degraded)
    }

    /// Resets every field to its default, recording a return to `Uninitialized`.
    ///
    /// The transition queue is preserved so outstanding transitions can still
    /// be drained after the reset.
    pub(crate) fn reset(&mut self) {
        let mut transitions = core::mem::take(&mut self.transitions);
        if self.state != State::Uninitialized {
            transitions.push((self.state, State::Uninitialized));
        }
        *self = Self {
            transitions,
            ..Self::default()
        };
    }
}

const fn valid(from: State, to: State) -> bool {
    matches!(
        (from, to),
        (State::Uninitialized | State::Degraded, State::Initializing)
            | (State::Initializing | State::Degraded, State::Operational)
            | (
                State::Initializing | State::Operational | State::Ready,
                State::Degraded
            )
            | (State::Operational, State::Ready)
    )
}
