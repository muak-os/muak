/// Shared netlink test fixtures for supervisor-oriented networkd tests.
use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use netlib::interface::{Ethernet, Name};
use netlib::link::{Failure, State};

/// Holds in-memory link and route state for deterministic tests.
#[derive(Debug, Default)]
pub(crate) struct MockInner {
    pub(crate) links: HashMap<String, MockLink>,
    pub(crate) next_index: u32,
    pub(crate) ipv4_addrs: HashMap<u32, HashSet<(Ipv4Addr, u8)>>,
    pub(crate) ipv6_addrs: HashMap<u32, HashSet<(Ipv6Addr, u8)>>,
    pub(crate) default_routes_v4: HashSet<Ipv4Addr>,
    pub(crate) default_routes_v6: HashSet<Ipv6Addr>,
}

/// Describes one mock link and its optional master bridge.
#[derive(Debug, Clone)]
pub(crate) struct MockLink {
    pub(crate) index: u32,
    pub(crate) mac: [u8; 6],
    pub(crate) up: bool,
    pub(crate) master_index: Option<u32>,
}

/// Implements `netlib::netlink::Ops` for deterministic tests.
#[derive(Clone, Debug)]
pub struct MockNetlinkOps {
    pub(crate) state: Arc<Mutex<MockInner>>,
}

impl MockNetlinkOps {
    /// Returns an empty mock backend.
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(MockInner {
                next_index: 1,
                ..MockInner::default()
            })),
        }
    }

    /// Adds a link with a deterministic index.
    pub fn add_link(&self, name: &str, mac: [u8; 6], up: bool) -> u32 {
        let mut state = self.state.lock().expect("mock lock poisoned");
        let index = state.next_index;
        state.next_index += 1;
        state.links.insert(
            name.to_string(),
            MockLink {
                index,
                mac,
                up,
                master_index: None,
            },
        );
        index
    }

    /// Sets the master bridge index for a named link.
    pub fn set_master(&self, name: &str, master_index: u32) {
        let mut state = self.state.lock().expect("mock lock poisoned");
        if let Some(link) = state.links.get_mut(name) {
            link.master_index = Some(master_index);
        }
    }
}

impl netlib::link::Ops for MockNetlinkOps {
    async fn exists(&self, name: &str) -> netlib::link::Result<bool> {
        Ok(self.state.lock().expect("lock").links.contains_key(name))
    }

    async fn index(&self, name: &str) -> netlib::link::Result<u32> {
        self.state
            .lock()
            .expect("lock")
            .links
            .get(name)
            .map(|link| link.index)
            .ok_or_else(|| Failure::NotFound(name.to_string()))
    }

    async fn ensure_up(&self, name: &str) -> netlib::link::Result<u32> {
        let mut state = self.state.lock().expect("lock");
        let link = state
            .links
            .get_mut(name)
            .ok_or_else(|| Failure::NotFound(name.to_string()))?;
        link.up = true;
        Ok(link.index)
    }

    async fn bring_up(&self, index: u32) -> netlib::link::Result<()> {
        if let Some(link) = self
            .state
            .lock()
            .expect("lock")
            .links
            .values_mut()
            .find(|link| link.index == index)
        {
            link.up = true;
        }
        Ok(())
    }

    async fn bring_down(&self, index: u32) -> netlib::link::Result<()> {
        if let Some(link) = self
            .state
            .lock()
            .expect("lock")
            .links
            .values_mut()
            .find(|link| link.index == index)
        {
            link.up = false;
        }
        Ok(())
    }

    async fn set_master(&self, slave: u32, master: u32) -> netlib::link::Result<()> {
        if let Some(link) = self
            .state
            .lock()
            .expect("lock")
            .links
            .values_mut()
            .find(|link| link.index == slave)
        {
            link.master_index = Some(master);
        }
        Ok(())
    }

    async fn delete(&self, index: u32) -> netlib::link::Result<()> {
        let mut state = self.state.lock().expect("lock");
        state.links.retain(|_, link| link.index != index);
        state.ipv4_addrs.remove(&index);
        state.ipv6_addrs.remove(&index);
        Ok(())
    }

    async fn probe_carriers(
        &self,
        interfaces: &[(u32, &str)],
        _timeout: Duration,
    ) -> HashMap<u32, bool> {
        let state = self.state.lock().expect("lock");
        interfaces
            .iter()
            .map(|(index, name)| (*index, state.links.get(*name).is_some_and(|link| link.up)))
            .collect()
    }
}

impl netlib::address::Ops for MockNetlinkOps {
    async fn ensure_ipv4(
        &self,
        index: u32,
        ip: Ipv4Addr,
        prefix: u8,
    ) -> netlib::address::Result<()> {
        self.state
            .lock()
            .expect("lock")
            .ipv4_addrs
            .entry(index)
            .or_default()
            .insert((ip, prefix));
        Ok(())
    }

    async fn find_ipv4(&self, index: u32) -> netlib::address::Result<Option<(Ipv4Addr, u8)>> {
        Ok(self
            .state
            .lock()
            .expect("lock")
            .ipv4_addrs
            .get(&index)
            .and_then(|addrs| addrs.iter().next().copied()))
    }

    async fn has_ipv4(&self, index: u32) -> netlib::address::Result<bool> {
        Ok(self
            .state
            .lock()
            .expect("lock")
            .ipv4_addrs
            .get(&index)
            .is_some_and(|addrs| !addrs.is_empty()))
    }

    async fn add_ipv4(&self, index: u32, ip: Ipv4Addr, prefix: u8) -> netlib::address::Result<()> {
        self.ensure_ipv4(index, ip, prefix).await
    }

    async fn remove_ipv4(&self, index: u32, ip: Ipv4Addr) -> netlib::address::Result<()> {
        if let Some(addrs) = self.state.lock().expect("lock").ipv4_addrs.get_mut(&index) {
            addrs.retain(|(addr, _)| *addr != ip);
        }
        Ok(())
    }

    async fn ensure_ipv6(
        &self,
        index: u32,
        ip: Ipv6Addr,
        prefix: u8,
    ) -> netlib::address::Result<()> {
        self.state
            .lock()
            .expect("lock")
            .ipv6_addrs
            .entry(index)
            .or_default()
            .insert((ip, prefix));
        Ok(())
    }

    async fn remove_ipv6(&self, index: u32, ip: Ipv6Addr) -> netlib::address::Result<()> {
        if let Some(addrs) = self.state.lock().expect("lock").ipv6_addrs.get_mut(&index) {
            addrs.retain(|(addr, _)| *addr != ip);
        }
        Ok(())
    }
}

impl netlib::route::Ops for MockNetlinkOps {
    async fn ensure_default_route(&self, gateway: Ipv4Addr) -> netlib::route::Result<()> {
        self.state
            .lock()
            .expect("lock")
            .default_routes_v4
            .insert(gateway);
        Ok(())
    }

    async fn ensure_default_route_v6(&self, gateway: Ipv6Addr) -> netlib::route::Result<()> {
        self.state
            .lock()
            .expect("lock")
            .default_routes_v6
            .insert(gateway);
        Ok(())
    }

    async fn remove_default_route_v6(&self, gateway: Ipv6Addr) -> netlib::route::Result<()> {
        self.state
            .lock()
            .expect("lock")
            .default_routes_v6
            .remove(&gateway);
        Ok(())
    }
}

impl netlib::bridge::Ops for MockNetlinkOps {
    async fn ensure_bridge(
        &self,
        bridge_name: &str,
        physical_iface: &str,
        _gateway: Option<Ipv4Addr>,
        _stp: bool,
    ) -> netlib::bridge::Result<()> {
        let mut state = self.state.lock().expect("lock");
        let bridge_index = if let Some(link) = state.links.get(bridge_name) {
            link.index
        } else {
            let index = state.next_index;
            state.next_index += 1;
            state.links.insert(
                bridge_name.to_string(),
                MockLink {
                    index,
                    mac: [0; 6],
                    up: true,
                    master_index: None,
                },
            );
            index
        };

        if let Some(link) = state.links.get_mut(physical_iface) {
            link.master_index = Some(bridge_index);
        }
        Ok(())
    }

    async fn attach_to_bridge(
        &self,
        iface_name: &str,
        bridge_name: &str,
    ) -> netlib::bridge::Result<()> {
        let bridge_index = netlib::link::Ops::index(self, bridge_name).await?;
        netlib::link::Ops::set_master(
            self,
            netlib::link::Ops::index(self, iface_name).await?,
            bridge_index,
        )
        .await?;
        Ok(())
    }
}

impl netlib::interface::Ops for MockNetlinkOps {
    async fn discover_ethernet(&self) -> netlib::interface::Result<Vec<Ethernet>> {
        let state = self.state.lock().expect("lock");
        let mut interfaces = Vec::with_capacity(state.links.len());
        for (name, link) in &state.links {
            interfaces.push(Ethernet::new(
                Name::new(name.clone())?,
                link.index,
                link.mac,
                link_state_kind(link.up),
            ));
        }
        Ok(interfaces)
    }
}

impl netlib::netlink::Ops for MockNetlinkOps {}

/// Returns the link state for the requested carrier flag.
fn link_state_kind(up: bool) -> State {
    if up { State::Up } else { State::Down }
}
