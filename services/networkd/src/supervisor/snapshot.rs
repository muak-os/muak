//! Network subsystem readiness, primary, and backup interfaces.

use anyhow::Result;
use netlink::link::ifname::Name;

use crate::statemachine::StateMachine as _;
use crate::supervisor::state::NetworkState;

/// Point-in-time view of the network subsystem's readiness and interface election.
#[derive(Debug)]
pub(super) struct NetworkSnapshot {
    pub state: NetworkState,
    pub primary: Option<Name>,
    pub backups: Vec<Name>,
}

impl NetworkSnapshot {
    /// Returns an empty snapshot in the uninitialized state.
    pub(super) fn empty() -> Self {
        Self {
            state: NetworkState::Uninitialized,
            primary: None,
            backups: Vec::new(),
        }
    }

    /// Returns true when the network is operational or ready.
    pub(super) fn is_operational(&self) -> bool {
        matches!(self.state, NetworkState::Operational | NetworkState::Ready)
    }

    /// Advances the network to `Operational` unless it is already operational or ready.
    pub(super) fn ensure_operational(&mut self) -> Result<()> {
        if self.is_operational() {
            return Ok(());
        }

        self.transition(NetworkState::Operational)
    }

    /// Advances the network to `Degraded` unless it is already degraded.
    pub(super) fn ensure_degraded(&mut self) -> Result<()> {
        if self.state == NetworkState::Degraded {
            return Ok(());
        }

        self.transition(NetworkState::Degraded)
    }

    /// Advances the network state, logging, and validating the transition.
    pub(super) fn transition(&mut self, to: NetworkState) -> Result<()> {
        let from = self.state.clone();
        self.state
            .transition(to)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        kmsg::info!("Network state: {} -> {}", from, self.state);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_snapshot_empty_defaults() {
        // ACT
        let snap = NetworkSnapshot::empty();

        // ASSERT
        assert_eq!(snap.state, NetworkState::Uninitialized);
        assert!(snap.primary.is_none());
        assert_eq!(snap.backups, Vec::<Name>::new());
    }

    #[test]
    fn ensure_operational_from_ready_is_noop() {
        // ARRANGE
        let mut snap = NetworkSnapshot::empty();
        snap.state = NetworkState::Ready;

        // ACT
        snap.ensure_operational()
            .expect("ready must already count as operational");

        // ASSERT
        assert_eq!(snap.state, NetworkState::Ready);
    }

    #[test]
    fn ensure_operational_from_operational_is_noop() {
        // ARRANGE
        let mut snap = NetworkSnapshot::empty();
        snap.state = NetworkState::Operational;

        // ACT
        snap.ensure_operational()
            .expect("operational must be a no-op");

        // ASSERT
        assert_eq!(snap.state, NetworkState::Operational);
    }

    #[test]
    fn ensure_operational_from_degraded_advances() {
        // ARRANGE
        let mut snap = NetworkSnapshot::empty();
        snap.state = NetworkState::Degraded;

        // ACT
        snap.ensure_operational()
            .expect("degraded must advance to operational");

        // ASSERT
        assert_eq!(snap.state, NetworkState::Operational);
    }

    #[test]
    fn ensure_operational_from_uninitialized_returns_error() {
        // ARRANGE
        let mut snap = NetworkSnapshot::empty();

        // ACT
        let result = snap.ensure_operational();

        // ASSERT
        result.expect_err("an uninitialized network cannot become operational");
        assert_eq!(snap.state, NetworkState::Uninitialized);
    }

    #[test]
    fn ensure_operational_from_initializing_advances() {
        // ARRANGE
        let mut snap = NetworkSnapshot::empty();
        snap.state = NetworkState::Initializing;

        // ACT
        snap.ensure_operational()
            .expect("initializing must advance to operational");

        // ASSERT
        assert_eq!(snap.state, NetworkState::Operational);
    }

    #[test]
    fn ensure_degraded_from_operational_degrades() {
        // ARRANGE
        let mut snap = NetworkSnapshot::empty();
        snap.state = NetworkState::Operational;

        // ACT
        snap.ensure_degraded().expect("operational must degrade");

        // ASSERT
        assert_eq!(snap.state, NetworkState::Degraded);
    }

    #[test]
    fn ensure_degraded_from_degraded_is_noop() {
        // ARRANGE
        let mut snap = NetworkSnapshot::empty();
        snap.state = NetworkState::Degraded;

        // ACT
        snap.ensure_degraded()
            .expect("degraded must already be a no-op");

        // ASSERT
        assert_eq!(snap.state, NetworkState::Degraded);
    }

    #[test]
    fn ensure_degraded_from_ready_degrades() {
        // ARRANGE
        let mut snap = NetworkSnapshot::empty();
        snap.state = NetworkState::Ready;

        // ACT
        snap.ensure_degraded().expect("ready must degrade");

        // ASSERT
        assert_eq!(snap.state, NetworkState::Degraded);
    }

    #[test]
    fn ensure_degraded_from_initializing_degrades() {
        // ARRANGE
        let mut snap = NetworkSnapshot::empty();
        snap.state = NetworkState::Initializing;

        // ACT
        snap.ensure_degraded().expect("initializing must degrade");

        // ASSERT
        assert_eq!(snap.state, NetworkState::Degraded);
    }

    #[test]
    fn ensure_degraded_from_uninitialized_returns_error() {
        // ARRANGE
        let mut snap = NetworkSnapshot::empty();

        // ACT
        let result = snap.ensure_degraded();

        // ASSERT
        result.expect_err("an uninitialized network cannot degrade");
        assert_eq!(snap.state, NetworkState::Uninitialized);
    }

    #[test]
    fn is_operational_reports_ready_and_operational() {
        // ARRANGE
        let mut snap = NetworkSnapshot::empty();

        // ACT
        snap.state = NetworkState::Operational;
        let operational = snap.is_operational();
        snap.state = NetworkState::Ready;
        let ready = snap.is_operational();
        snap.state = NetworkState::Degraded;
        let degraded = snap.is_operational();
        snap.state = NetworkState::Initializing;
        let initializing = snap.is_operational();
        snap.state = NetworkState::Uninitialized;
        let uninitialized = snap.is_operational();

        // ASSERT
        assert!(operational, "operational must count as operational");
        assert!(ready, "ready must count as operational");
        assert!(!degraded, "degraded is not operational");
        assert!(!initializing, "initializing is not operational");
        assert!(!uninitialized, "uninitialized is not operational");
    }
}
