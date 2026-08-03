//! Snapshot of a single network interface's runtime state.

use anyhow::Result;
use netlib::address::{IpConfig, Ipv6Config};
use netlib::interface::Name;
use netlib::link::State;

use crate::dhcp::{Lease, State as DhcpState};
use crate::interface::state::Lifecycle;
use crate::statemachine::StateMachine as _;

/// Point-in-time view of one interface's configuration and link status.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub name: Name,
    pub state: Lifecycle,
    pub index: u32,
    pub mac: [u8; 6],
    pub link: State,
    pub ip: Option<IpConfig>,
    pub lease: Option<Lease>,
    pub dhcp_state: Option<DhcpState>,
    pub ipv6: Option<Ipv6Config>,
    pub l3_owner: Name,
}

impl Snapshot {
    /// Advances this interface's life cycle state, logging and validating the transition.
    ///
    /// # Errors
    ///
    /// Returns an error if the requested state transition is not valid.
    pub fn transition(&mut self, to: Lifecycle) -> Result<()> {
        let from = self.state.clone();
        self.state
            .transition(to)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        kmsg::info!("Interface {} state: {} -> {}", self.name, from, self.state);
        Ok(())
    }
}
