//! Snapshot of a single network interface's runtime state.

use anyhow::Result;
use netlib::address::{IpConfig, Ipv6Config};
use netlib::interface::InterfaceName;
use netlib::link::LinkStateKind;

use crate::dhcp::{DhcpLease, DhcpState};
use crate::interface::state::InterfaceState;
use crate::statemachine::StateMachine;

/// Point-in-time view of one interface's configuration and link status.
#[derive(Debug, Clone)]
pub struct InterfaceSnapshot {
    pub name: InterfaceName,
    pub state: InterfaceState,
    pub index: u32,
    pub mac: [u8; 6],
    pub link: LinkStateKind,
    pub ip: Option<IpConfig>,
    pub lease: Option<DhcpLease>,
    pub dhcp_state: Option<DhcpState>,
    pub ipv6: Option<Ipv6Config>,
}

impl InterfaceSnapshot {
    /// Advances this interface's life cycle state, logging and validating the transition.
    pub fn transition(&mut self, to: InterfaceState) -> Result<()> {
        let from = self.state.clone();
        self.state
            .transition(to)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        kmsg::info!("Interface {} state: {} -> {}", self.name, from, self.state);
        Ok(())
    }
}
