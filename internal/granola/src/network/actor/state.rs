use rtnetlink::Handle;
use std::collections::HashMap;
use tokio::sync::watch;

use crate::network::model::{InterfaceSnapshot, NetworkSnapshot};

pub struct NetworkActor {
    pub(super) handle: Handle,
    pub(super) state: NetworkSnapshot,
    pub(super) iface_map: HashMap<String, InterfaceSnapshot>,
    pub(super) watch_tx: watch::Sender<NetworkSnapshot>,
}

impl NetworkActor {
    pub fn new(handle: Handle, watch_tx: watch::Sender<NetworkSnapshot>) -> Self {
        Self {
            handle,
            state: NetworkSnapshot::empty(),
            iface_map: HashMap::new(),
            watch_tx,
        }
    }

    pub(super) fn publish_state(&self) {
        let _ = self.watch_tx.send(self.state.clone());
    }

    pub(super) fn sync_and_publish(&mut self) {
        self.state.interfaces = self.iface_map.values().cloned().collect();
        self.publish_state();
    }

    pub(super) fn get_interface(&self, name: &str) -> Option<&InterfaceSnapshot> {
        self.iface_map.get(name)
    }

    pub(super) fn get_interface_mut(&mut self, name: &str) -> Option<&mut InterfaceSnapshot> {
        self.iface_map.get_mut(name)
    }

    pub(super) fn get_interface_by_index(&self, index: u32) -> Option<&InterfaceSnapshot> {
        self.iface_map.values().find(|i| i.index == index)
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
}
