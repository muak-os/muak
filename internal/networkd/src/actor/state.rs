use rtnetlink::Handle;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::config::NetworkConfig;
use crate::model::{InterfaceSnapshot, NetworkSnapshot};

/// Grace period after becoming Ready to filter spurious failover events
/// from queued bridge setup events
pub const READY_GRACE_PERIOD_SECS: u64 = 5;

pub struct NetworkActor {
    pub(super) handle: Handle,
    pub(super) config: NetworkConfig,
    pub(super) state: NetworkSnapshot,
    pub(super) iface_map: HashMap<String, InterfaceSnapshot>,
    pub(super) watch_tx: watch::Sender<NetworkSnapshot>,
    pub(super) renewal_tasks: HashMap<String, Vec<JoinHandle<()>>>,
    pub(super) connectivity_task: Option<JoinHandle<()>>,
    /// Timestamp when network became Ready - used to ignore queued init events
    pub(super) ready_at: Option<Instant>,
}

impl NetworkActor {
    pub fn new(
        handle: Handle,
        watch_tx: watch::Sender<NetworkSnapshot>,
        config: NetworkConfig,
    ) -> Self {
        Self {
            handle,
            config,
            state: NetworkSnapshot::empty(),
            iface_map: HashMap::new(),
            watch_tx,
            renewal_tasks: HashMap::new(),
            connectivity_task: None,
            ready_at: None,
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
}
