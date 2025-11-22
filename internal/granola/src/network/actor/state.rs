use rtnetlink::Handle;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::network::model::{InterfaceSnapshot, NetworkSnapshot};

pub struct NetworkActor {
    pub(super) handle: Handle,
    pub(super) state: NetworkSnapshot,
    pub(super) iface_map: HashMap<String, InterfaceSnapshot>,
    pub(super) watch_tx: watch::Sender<NetworkSnapshot>,
    pub(super) renewal_tasks: HashMap<String, Vec<JoinHandle<()>>>,
    cmd_tx: tokio::sync::mpsc::Sender<super::commands::NetworkCommand>,
}

impl NetworkActor {
    pub fn new(
        handle: Handle,
        watch_tx: watch::Sender<NetworkSnapshot>,
        cmd_tx: tokio::sync::mpsc::Sender<super::commands::NetworkCommand>,
    ) -> Self {
        Self {
            handle,
            state: NetworkSnapshot::empty(),
            iface_map: HashMap::new(),
            watch_tx,
            renewal_tasks: HashMap::new(),
            cmd_tx,
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
        if let Some(tasks) = self.renewal_tasks.remove(iface) {
            for task in tasks {
                task.abort();
            }
        }
    }

    pub(super) fn get_command_sender(&self) -> tokio::sync::mpsc::Sender<super::commands::NetworkCommand> {
        self.cmd_tx.clone()
    }
}
