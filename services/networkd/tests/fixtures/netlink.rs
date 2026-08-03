/// Shared netlink test fixtures for supervisor-oriented networkd tests.
extern crate alloc;

use alloc::sync::Arc;
use core::net::{Ipv4Addr, Ipv6Addr};
use core::time::Duration;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use netlib::address;
use netlib::bridge;
use netlib::interface;
use netlib::interface::{Ethernet, Name};
use netlib::link;
use netlib::link::{Failure, State};
use netlib::netlink;
use netlib::route;

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
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let index = state.next_index;
        state.next_index = state.next_index.saturating_add(1);
        state.links.insert(
            name.to_owned(),
            MockLink {
                index,
                mac,
                up,
                master_index: None,
            },
        );
        index
    }
}

impl link::Ops for MockNetlinkOps {
    async fn exists(&self, name: &str) -> link::Result<bool> {
        Ok(self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .links
            .contains_key(name))
    }

    async fn index(&self, name: &str) -> link::Result<u32> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .links
            .get(name)
            .map(|link| link.index)
            .ok_or_else(|| Failure::NotFound(name.to_owned()))
    }

    async fn ensure_up(&self, name: &str) -> link::Result<u32> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let link = state
            .links
            .get_mut(name)
            .ok_or_else(|| Failure::NotFound(name.to_owned()))?;
        link.up = true;
        Ok(link.index)
    }

    async fn bring_up(&self, index: u32) -> link::Result<()> {
        if let Some(link) = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .links
            .values_mut()
            .find(|link| link.index == index)
        {
            link.up = true;
        }
        Ok(())
    }

    async fn bring_down(&self, index: u32) -> link::Result<()> {
        if let Some(link) = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .links
            .values_mut()
            .find(|link| link.index == index)
        {
            link.up = false;
        }
        Ok(())
    }

    async fn set_master(&self, slave_index: u32, master_index: u32) -> link::Result<()> {
        if let Some(link) = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .links
            .values_mut()
            .find(|link| link.index == slave_index)
        {
            link.master_index = Some(master_index);
        }
        Ok(())
    }

    async fn delete(&self, index: u32) -> link::Result<()> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        interfaces
            .iter()
            .map(|&(index, name)| (index, state.links.get(name).is_some_and(|link| link.up)))
            .collect()
    }
}

impl address::Ops for MockNetlinkOps {
    async fn ensure_ipv4(&self, index: u32, ip: Ipv4Addr, prefix: u8) -> address::Result<()> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .ipv4_addrs
            .entry(index)
            .or_default()
            .insert((ip, prefix));
        Ok(())
    }

    async fn find_ipv4(&self, index: u32) -> address::Result<Option<(Ipv4Addr, u8)>> {
        Ok(self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .ipv4_addrs
            .get(&index)
            .and_then(|addrs| addrs.iter().next().copied()))
    }

    async fn has_ipv4(&self, index: u32) -> address::Result<bool> {
        Ok(self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .ipv4_addrs
            .get(&index)
            .is_some_and(|addrs| !addrs.is_empty()))
    }

    async fn add_ipv4(&self, index: u32, ip: Ipv4Addr, prefix: u8) -> address::Result<()> {
        self.ensure_ipv4(index, ip, prefix).await
    }

    async fn remove_ipv4(&self, index: u32, ip: Ipv4Addr) -> address::Result<()> {
        if let Some(addrs) = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .ipv4_addrs
            .get_mut(&index)
        {
            addrs.retain(|&(addr, _)| addr != ip);
        }
        Ok(())
    }

    async fn ensure_ipv6(&self, index: u32, ip: Ipv6Addr, prefix: u8) -> address::Result<()> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .ipv6_addrs
            .entry(index)
            .or_default()
            .insert((ip, prefix));
        Ok(())
    }

    async fn remove_ipv6(&self, index: u32, ip: Ipv6Addr) -> address::Result<()> {
        if let Some(addrs) = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .ipv6_addrs
            .get_mut(&index)
        {
            addrs.retain(|&(addr, _)| addr != ip);
        }
        Ok(())
    }
}

impl route::Ops for MockNetlinkOps {
    async fn ensure_default_route(&self, gateway: Ipv4Addr) -> route::Result<()> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .default_routes_v4
            .insert(gateway);
        Ok(())
    }

    async fn ensure_default_route_v6(&self, gateway: Ipv6Addr) -> route::Result<()> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .default_routes_v6
            .insert(gateway);
        Ok(())
    }

    async fn remove_default_route_v6(&self, gateway: Ipv6Addr) -> route::Result<()> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .default_routes_v6
            .remove(&gateway);
        Ok(())
    }
}

impl bridge::Ops for MockNetlinkOps {
    async fn ensure_bridge(
        &self,
        bridge_name: &str,
        physical_iface: &str,
        _gateway: Option<Ipv4Addr>,
        _stp: bool,
    ) -> bridge::Result<()> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let bridge_index = if let Some(link) = state.links.get(bridge_name) {
            link.index
        } else {
            let index = state.next_index;
            state.next_index = state.next_index.saturating_add(1);
            state.links.insert(
                bridge_name.to_owned(),
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

    async fn attach_to_bridge(&self, iface_name: &str, bridge_name: &str) -> bridge::Result<()> {
        let bridge_index = link::Ops::index(self, bridge_name).await?;
        link::Ops::set_master(
            self,
            link::Ops::index(self, iface_name).await?,
            bridge_index,
        )
        .await?;
        Ok(())
    }
}

impl interface::Ops for MockNetlinkOps {
    async fn discover_ethernet(&self) -> interface::Result<Vec<Ethernet>> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut links: Vec<_> = state.links.iter().collect();
        links.sort_by(|left, right| left.0.cmp(right.0));
        let mut interfaces = Vec::with_capacity(links.len());
        for (name, link) in links {
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

impl netlink::Ops for MockNetlinkOps {}

/// Returns the link state for the requested carrier flag.
fn link_state_kind(up: bool) -> State {
    if up { State::Up } else { State::Down }
}
