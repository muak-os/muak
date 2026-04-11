//! TAP interface operations.

use config::InterfaceKind;
use netlib::link::LinkStateKind;
use netlib::tap;

use super::state::{InterfaceSnapshot, NetworkActor};

impl NetworkActor {
    /// Returns the name of the first configured bridge interface, if any.
    pub(super) fn bridge_name(&self) -> Option<&str> {
        config::network()
            .interfaces
            .iter()
            .find(|i| i.kind == InterfaceKind::Bridge)
            .map(|i| i.name.as_str())
    }

    /// Adds a TAP interface with the given name, enslaved to the bridge, and returns its snapshot.
    pub(super) async fn add_tap(&mut self, name: &str) -> anyhow::Result<InterfaceSnapshot> {
        kmsg::info!("Adding TAP interface: {}", name);

        let bridge_name = self
            .bridge_name()
            .ok_or_else(|| anyhow::anyhow!("no bridge interface configured"))?
            .to_string();

        let index = tap::setup_on_bridge(&self.handle, name, &bridge_name).await?;

        let snapshot = InterfaceSnapshot {
            name: name.to_string(),
            index,
            mac: [0, 0, 0, 0, 0, 0],
            link: LinkStateKind::Up,
            ip: None,
            lease: None,
            ipv6: None,
        };

        self.insert_interface(snapshot.clone());
        self.sync_and_publish();

        kmsg::info!("TAP interface added: {}", name);
        Ok(snapshot)
    }

    /// Deletes the TAP interface with the given name and updates the snapshot.
    pub(super) async fn delete_tap(&mut self, name: &str) -> anyhow::Result<()> {
        kmsg::info!("Deleting TAP interface: {}", name);

        tap::remove(&self.handle, name).await?;
        self.remove_interface(name);
        self.sync_and_publish();

        kmsg::info!("TAP interface deleted: {}", name);
        Ok(())
    }
}
