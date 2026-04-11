//! State types for the network actor and its view of the system network topology.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::Arc;

use anyhow::Result;
use netlib::address::{IpConfig, Ipv6Config};
use netlib::link::LinkStateKind;
use rtnetlink::Handle;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::dhcp::DhcpLease;

#[derive(Debug, Clone, PartialEq)]
pub enum NetworkStateKind {
    Uninitialized,
    Initializing,
    Operational,
    Ready,
    Degraded,
}

#[derive(Debug, Clone)]
pub struct InterfaceSnapshot {
    pub name: String,
    pub index: u32,
    pub mac: [u8; 6],
    pub link: LinkStateKind,
    pub ip: Option<IpConfig>,
    pub lease: Option<DhcpLease>,
    pub ipv6: Option<Ipv6Config>,
}

#[derive(Debug, Clone)]
pub struct NetworkSnapshot {
    pub state: NetworkStateKind,
    pub primary: Option<String>,
    pub backups: Vec<String>,
    pub interfaces: Vec<Arc<InterfaceSnapshot>>,
    pub ipv6: bool,
}

impl NetworkSnapshot {
    /// Returns an empty snapshot in the uninitialized state.
    pub fn empty() -> Self {
        Self {
            state: NetworkStateKind::Uninitialized,
            primary: None,
            backups: Vec::new(),
            interfaces: Vec::new(),
            ipv6: false,
        }
    }
}

pub struct NetworkActor {
    pub(super) handle: Handle,
    pub(super) state: NetworkSnapshot,
    pub(super) iface_map: HashMap<String, InterfaceSnapshot>,
    pub(super) watch_tx: watch::Sender<NetworkSnapshot>,
    pub(super) renewal_tasks: HashMap<String, Vec<JoinHandle<()>>>,
}

impl NetworkActor {
    pub fn new(handle: Handle, watch_tx: watch::Sender<NetworkSnapshot>) -> Self {
        Self {
            handle,
            state: NetworkSnapshot::empty(),
            iface_map: HashMap::new(),
            watch_tx,
            renewal_tasks: HashMap::new(),
        }
    }

    pub(super) fn publish_state(&self) {
        let _ = self.watch_tx.send(self.state.clone());
    }

    pub(super) fn sync_and_publish(&mut self) {
        self.state.interfaces = self
            .iface_map
            .values()
            .map(|iface| Arc::new(iface.clone()))
            .collect();
        self.publish_state();
    }

    pub(super) fn get_interface(&self, name: &str) -> Option<&InterfaceSnapshot> {
        self.iface_map.get(name)
    }

    pub(super) fn get_interface_mut(&mut self, name: &str) -> Option<&mut InterfaceSnapshot> {
        self.iface_map.get_mut(name)
    }

    pub(super) fn insert_interface(&mut self, iface: InterfaceSnapshot) {
        self.iface_map.insert(iface.name.clone(), iface);
    }

    pub(super) fn remove_interface(&mut self, name: &str) -> Option<InterfaceSnapshot> {
        self.iface_map.remove(name)
    }

    pub(super) fn has_interface(&self, name: &str) -> bool {
        self.iface_map.contains_key(name)
    }

    pub(super) fn track_renewal_task(&mut self, iface: String, task: JoinHandle<()>) {
        self.renewal_tasks.entry(iface).or_default().push(task);
    }

    pub(super) fn cancel_renewal_tasks(&mut self, iface: &str) {
        let Some(tasks) = self.renewal_tasks.remove(iface) else {
            return;
        };
        for task in tasks {
            task.abort();
        }
    }

    pub(super) fn get_primary_name(&self) -> Result<String> {
        self.state
            .primary
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no primary interface"))
    }

    pub(super) fn extract_lease_mac_and_gateway(
        &self,
        iface_name: &str,
    ) -> Result<(DhcpLease, [u8; 6], Option<Ipv4Addr>)> {
        let iface = self
            .get_interface(iface_name)
            .ok_or_else(|| anyhow::anyhow!("interface not found: {}", iface_name))?;

        let lease = iface
            .lease
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no DHCP lease on {}", iface_name))?;

        let gateway = iface.ip.as_ref().and_then(|ip| ip.gateway);

        Ok((lease, iface.mac, gateway))
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::*;

    #[test]
    fn dhcp_lease_expiry_is_obtained_plus_lease_time() {
        // ARRANGE
        let obtained = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let lease = DhcpLease {
            obtained_at: obtained,
            lease_time: Duration::from_secs(3600),
            renewal_time: Duration::from_secs(1800),
            rebind_time: Duration::from_secs(3150),
        };
        let expected = obtained + Duration::from_secs(3600);

        // ACT
        let result = lease.expiry();

        // ASSERT
        assert_eq!(result, expected);
    }

    #[test]
    fn dhcp_lease_expiry_at_epoch() {
        // ARRANGE
        let lease = DhcpLease {
            obtained_at: SystemTime::UNIX_EPOCH,
            lease_time: Duration::from_secs(86400),
            renewal_time: Duration::from_secs(43200),
            rebind_time: Duration::from_secs(75600),
        };
        let expected = SystemTime::UNIX_EPOCH + Duration::from_secs(86400);

        // ACT
        let result = lease.expiry();

        // ASSERT
        assert_eq!(result, expected);
    }

    #[test]
    fn network_snapshot_empty_defaults() {
        // ACT
        let snap = NetworkSnapshot::empty();

        // ASSERT
        assert_eq!(snap.state, NetworkStateKind::Uninitialized);
        assert!(snap.primary.is_none());
        assert!(snap.backups.is_empty());
        assert!(snap.interfaces.is_empty());
        assert!(!snap.ipv6);
    }
}
