//! Aggregate snapshot of the network subsystem and all known interfaces.

use std::sync::Arc;

use anyhow::Result;
use netlib::interface::Name;

use crate::interface::snapshot::InterfaceSnapshot;
use crate::statemachine::StateMachine;
use crate::supervisor::state::NetworkState;

/// Point-in-time view of the entire network subsystem.
#[derive(Debug, Clone)]
pub struct NetworkSnapshot {
    pub state: NetworkState,
    pub primary: Option<Name>,
    pub backups: Vec<Name>,
    pub interfaces: Vec<Arc<InterfaceSnapshot>>,
}

impl NetworkSnapshot {
    /// Returns an empty snapshot in the uninitialized state.
    pub fn empty() -> Self {
        Self {
            state: NetworkState::Uninitialized,
            primary: None,
            backups: Vec::new(),
            interfaces: Vec::new(),
        }
    }

    /// Advances the network state, logging and validating the transition.
    pub fn transition(&mut self, to: NetworkState) -> Result<()> {
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
        assert!(snap.backups.is_empty());
        assert!(snap.interfaces.is_empty());
    }
}
