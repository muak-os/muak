//! Shared netlink test fixtures for interface controller tests.

extern crate alloc;

use alloc::sync::Arc;
use core::future::Future;
use core::net::{Ipv4Addr, Ipv6Addr};
use core::time::Duration;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use netdev::interface::{self, Ethernet};
use netlink::link::device::Device;
use netlink::link::ifname::Name;
use netlink::link::state::State;
use netlink::link::{self, Failure};
use netlink::{address, bridge, route};

/// Holds in-memory link, address and route state for deterministic tests.
#[derive(Debug, Default)]
pub(crate) struct MockInner {
    pub(crate) links: HashMap<String, MockLink>,
    pub(crate) next_index: u32,
    pub(crate) ipv4_addrs: HashMap<u32, HashSet<(Ipv4Addr, u8)>>,
    pub(crate) ipv6_addrs: HashMap<u32, HashSet<(Ipv6Addr, u8)>>,
    pub(crate) default_routes_v4: HashSet<Ipv4Addr>,
    pub(crate) default_routes_v6: HashSet<Ipv6Addr>,
}

/// Describes one mock link.
#[derive(Debug, Clone)]
pub(crate) struct MockLink {
    pub(crate) index: u32,
    pub(crate) mac: [u8; 6],
    pub(crate) up: bool,
    pub(crate) master_index: Option<u32>,
}

/// Implements `netdev::Ops` for deterministic tests.
#[derive(Clone, Debug)]
pub(crate) struct MockNetlinkOps {
    state: Arc<Mutex<MockInner>>,
}

impl MockNetlinkOps {
    /// Returns an empty mock backend.
    pub(crate) fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(MockInner {
                next_index: 1,
                ..MockInner::default()
            })),
        }
    }

    /// Locks the inner state, tolerating a poisoned mutex.
    pub(crate) fn lock(&self) -> std::sync::MutexGuard<'_, MockInner> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Adds a link with a deterministic index.
    pub(crate) fn add_link(&self, name: &str, mac: [u8; 6], up: bool) -> u32 {
        let mut state = self.lock();
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
    fn devices(&self) -> impl Future<Output = link::Result<Vec<Device>>> {
        let state = self.lock();
        let devices = state
            .links
            .iter()
            .map(|(name, link)| Device {
                index: link.index,
                name: name.clone(),
                mac: link.mac,
                is_virtual: false,
                admin_up: link.up,
                carrier: link.up,
                master: link.master_index,
            })
            .collect();

        std::future::ready(Ok(devices))
    }

    fn exists(&self, name: &str) -> impl Future<Output = link::Result<bool>> {
        std::future::ready(Ok(self.lock().links.contains_key(name)))
    }

    fn index(&self, name: &str) -> impl Future<Output = link::Result<u32>> {
        let result = self
            .lock()
            .links
            .get(name)
            .map(|link| link.index)
            .ok_or_else(|| Failure::NotFound(name.to_owned()));

        std::future::ready(result)
    }

    fn ensure_up(&self, name: &str) -> impl Future<Output = link::Result<u32>> {
        let result = match self.lock().links.get_mut(name) {
            Some(link) => {
                link.up = true;
                Ok(link.index)
            }
            None => Err(Failure::NotFound(name.to_owned())),
        };

        std::future::ready(result)
    }

    fn bring_up(&self, index: u32) -> impl Future<Output = link::Result<()>> {
        if let Some(link) = self
            .lock()
            .links
            .values_mut()
            .find(|link| link.index == index)
        {
            link.up = true;
        }

        std::future::ready(Ok(()))
    }

    fn bring_down(&self, index: u32) -> impl Future<Output = link::Result<()>> {
        if let Some(link) = self
            .lock()
            .links
            .values_mut()
            .find(|link| link.index == index)
        {
            link.up = false;
        }

        std::future::ready(Ok(()))
    }

    fn set_master(
        &self,
        slave_index: u32,
        master_index: u32,
    ) -> impl Future<Output = link::Result<()>> {
        if let Some(link) = self
            .lock()
            .links
            .values_mut()
            .find(|link| link.index == slave_index)
        {
            link.master_index = Some(master_index);
        }

        std::future::ready(Ok(()))
    }

    fn master(&self, index: u32) -> impl Future<Output = link::Result<Option<u32>>> {
        let result = self
            .lock()
            .links
            .values()
            .find(|link| link.index == index)
            .and_then(|link| link.master_index);

        std::future::ready(Ok(result))
    }

    fn detach(&self, index: u32) -> impl Future<Output = link::Result<()>> {
        if let Some(link) = self
            .lock()
            .links
            .values_mut()
            .find(|link| link.index == index)
        {
            link.master_index = None;
        }

        std::future::ready(Ok(()))
    }

    fn set_address(&self, index: u32, mac: [u8; 6]) -> impl Future<Output = link::Result<()>> {
        if let Some(link) = self
            .lock()
            .links
            .values_mut()
            .find(|link| link.index == index)
        {
            link.mac = mac;
        }

        std::future::ready(Ok(()))
    }

    fn delete(&self, index: u32) -> impl Future<Output = link::Result<()>> {
        let mut state = self.lock();
        state.links.retain(|_, link| link.index != index);
        state.ipv4_addrs.remove(&index);
        state.ipv6_addrs.remove(&index);

        std::future::ready(Ok(()))
    }

    fn probe_carriers(
        &self,
        interfaces: &[(u32, &str)],
        _timeout: Duration,
    ) -> impl Future<Output = link::Result<HashMap<u32, bool>>> {
        let state = self.lock();
        let result = interfaces
            .iter()
            .map(|&(index, name)| (index, state.links.get(name).is_some_and(|link| link.up)))
            .collect();

        std::future::ready(Ok(result))
    }
}

impl address::Ops for MockNetlinkOps {
    fn ensure_ipv4(
        &self,
        index: u32,
        ip: Ipv4Addr,
        prefix: u8,
    ) -> impl Future<Output = address::Result<()>> {
        self.lock()
            .ipv4_addrs
            .entry(index)
            .or_default()
            .insert((ip, prefix));

        std::future::ready(Ok(()))
    }

    fn add_ipv4(
        &self,
        index: u32,
        ip: Ipv4Addr,
        prefix: u8,
    ) -> impl Future<Output = address::Result<()>> {
        self.ensure_ipv4(index, ip, prefix)
    }

    fn find_ipv4(
        &self,
        index: u32,
    ) -> impl Future<Output = address::Result<Option<(Ipv4Addr, u8)>>> {
        let result = self
            .lock()
            .ipv4_addrs
            .get(&index)
            .and_then(|addrs| addrs.iter().next().copied());

        std::future::ready(Ok(result))
    }

    fn has_ipv4(&self, index: u32) -> impl Future<Output = address::Result<bool>> {
        let result = self
            .lock()
            .ipv4_addrs
            .get(&index)
            .is_some_and(|addrs| !addrs.is_empty());

        std::future::ready(Ok(result))
    }

    fn remove_ipv4(&self, index: u32, ip: Ipv4Addr) -> impl Future<Output = address::Result<()>> {
        if let Some(addrs) = self.lock().ipv4_addrs.get_mut(&index) {
            addrs.retain(|&(addr, _)| addr != ip);
        }

        std::future::ready(Ok(()))
    }

    fn ensure_ipv6(
        &self,
        index: u32,
        ip: Ipv6Addr,
        prefix: u8,
    ) -> impl Future<Output = address::Result<()>> {
        self.lock()
            .ipv6_addrs
            .entry(index)
            .or_default()
            .insert((ip, prefix));

        std::future::ready(Ok(()))
    }

    fn remove_ipv6(&self, index: u32, ip: Ipv6Addr) -> impl Future<Output = address::Result<()>> {
        if let Some(addrs) = self.lock().ipv6_addrs.get_mut(&index) {
            addrs.retain(|&(addr, _)| addr != ip);
        }

        std::future::ready(Ok(()))
    }
}

impl route::Ops for MockNetlinkOps {
    fn ensure_default_route(&self, gateway: Ipv4Addr) -> impl Future<Output = route::Result<bool>> {
        let inserted = self.lock().default_routes_v4.insert(gateway);

        std::future::ready(Ok(inserted))
    }

    fn ensure_default_route_v6(
        &self,
        gateway: Ipv6Addr,
    ) -> impl Future<Output = route::Result<bool>> {
        let inserted = self.lock().default_routes_v6.insert(gateway);

        std::future::ready(Ok(inserted))
    }

    fn remove_default_route_v6(
        &self,
        gateway: Ipv6Addr,
    ) -> impl Future<Output = route::Result<()>> {
        self.lock().default_routes_v6.remove(&gateway);

        std::future::ready(Ok(()))
    }
}

impl bridge::Ops for MockNetlinkOps {
    fn ensure_bridge(
        &self,
        _bridge_name: &str,
        _port_name: &str,
        _stp: bool,
    ) -> impl Future<Output = bridge::Result<bridge::Bridge>> {
        std::future::ready(Ok(bridge::Bridge {
            mac: [0; 6],
            link: State::Down,
        }))
    }
}

impl interface::Ops for MockNetlinkOps {
    fn discover_ethernet(&self) -> impl Future<Output = interface::Result<Vec<Ethernet>>> {
        let state = self.lock();
        let mut links: Vec<_> = state.links.iter().collect();
        links.sort_by(|left, right| left.0.cmp(right.0));
        let interfaces = links
            .into_iter()
            .map(|(name, link)| {
                Name::new(name.clone())
                    .map_err(Into::into)
                    .map(|name| Ethernet::new(name, link.index, link.mac, link_state(link.up)))
            })
            .collect();

        std::future::ready(interfaces)
    }
}

impl netdev::Ops for MockNetlinkOps {}

const fn link_state(up: bool) -> State {
    if up { State::Up } else { State::Down }
}
