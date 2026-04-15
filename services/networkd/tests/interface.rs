//! Integration tests for the per-interface actor.

#[path = "interface/actor.rs"]
mod actor;
#[path = "interface/bridge.rs"]
mod bridge;
#[path = "interface/dhcp.rs"]
mod dhcp;
#[path = "interface/link.rs"]
mod link;
#[path = "interface/slaac.rs"]
mod slaac;
#[path = "interface/static.rs"]
mod r#static;

use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use anyhow::Result;
use netlib::interface::{Interface, InterfaceName};
use netlib::link::LinkStateKind;
use netlib::ops::{AddressOps, BridgeOps, InterfaceOps, LinkOps, NetlinkOps, RouteOps};
use networkd::dhcp::DhcpConnector;
use networkd::interface::snapshot::InterfaceSnapshot;
use networkd::interface::state::InterfaceState;
use networkd::interface::{InterfaceActor, InterfaceCommand};

#[derive(Clone, Default)]
struct MockDhcpConnector;

impl DhcpConnector for MockDhcpConnector {
    async fn create(&self, _interface: &str) -> Result<tokio::net::UdpSocket> {
        Ok(tokio::net::UdpSocket::bind("127.0.0.1:0").await?)
    }
}

#[derive(Debug, Default)]
struct MockInner {
    links: HashMap<String, MockLink>,
    next_index: u32,
    ipv4_addrs: HashMap<u32, HashSet<(Ipv4Addr, u8)>>,
    ipv6_addrs: HashMap<u32, HashSet<(Ipv6Addr, u8)>>,
    default_routes_v4: HashSet<Ipv4Addr>,
    default_routes_v6: HashSet<Ipv6Addr>,
}

impl MockInner {
    fn link_by_index_mut(&mut self, index: u32) -> Option<&mut MockLink> {
        self.links.values_mut().find(|l| l.index == index)
    }
}

#[derive(Debug, Clone)]
struct MockLink {
    index: u32,
    mac: [u8; 6],
    up: bool,
    master_index: Option<u32>,
}

/// In-memory implementation of `NetlinkOps` for deterministic testing.
#[derive(Clone, Debug)]
pub struct MockNetlinkOps {
    state: Arc<Mutex<MockInner>>,
}

impl MockNetlinkOps {
    /// Creates an empty mock with no links.
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(MockInner {
                next_index: 1,
                ..MockInner::default()
            })),
        }
    }

    fn lock(&self) -> MutexGuard<'_, MockInner> {
        self.state.lock().expect("mock lock poisoned")
    }

    /// Adds a pre-existing link so `discover_ethernet` will return it.
    pub fn add_link(&self, name: &str, mac: [u8; 6], up: bool) -> u32 {
        let mut s = self.lock();
        let index = s.next_index;
        s.next_index += 1;
        s.links.insert(
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

    /// Returns a snapshot of IPv4 addresses for an index.
    pub fn ipv4_addrs(&self, index: u32) -> HashSet<(Ipv4Addr, u8)> {
        self.lock()
            .ipv4_addrs
            .get(&index)
            .cloned()
            .unwrap_or_default()
    }

    /// Returns a snapshot of IPv6 addresses for an index.
    pub fn ipv6_addrs(&self, index: u32) -> HashSet<(Ipv6Addr, u8)> {
        self.lock()
            .ipv6_addrs
            .get(&index)
            .cloned()
            .unwrap_or_default()
    }

    /// Returns true if the given IPv4 gateway is in the default route table.
    pub fn has_default_route_v4(&self, gw: Ipv4Addr) -> bool {
        self.lock().default_routes_v4.contains(&gw)
    }

    /// Returns true if the given IPv6 gateway is in the default route table.
    pub fn has_default_route_v6(&self, gw: Ipv6Addr) -> bool {
        self.lock().default_routes_v6.contains(&gw)
    }
}

fn link_state_kind(up: bool) -> LinkStateKind {
    if up {
        LinkStateKind::Up
    } else {
        LinkStateKind::Down
    }
}

impl LinkOps for MockNetlinkOps {
    async fn link_exists(&self, name: &str) -> netlib::link::Result<bool> {
        Ok(self.lock().links.contains_key(name))
    }

    async fn get_link_index(&self, name: &str) -> netlib::link::Result<u32> {
        self.lock()
            .links
            .get(name)
            .map(|l| l.index)
            .ok_or_else(|| netlib::link::Error::NotFound(name.to_string()))
    }

    async fn ensure_link_up(&self, name: &str) -> netlib::link::Result<u32> {
        let mut s = self.lock();
        let link = s
            .links
            .get_mut(name)
            .ok_or_else(|| netlib::link::Error::NotFound(name.to_string()))?;
        link.up = true;
        Ok(link.index)
    }

    async fn bring_link_up(&self, index: u32) -> netlib::link::Result<()> {
        if let Some(link) = self.lock().link_by_index_mut(index) {
            link.up = true;
        }
        Ok(())
    }

    async fn bring_link_down(&self, index: u32) -> netlib::link::Result<()> {
        if let Some(link) = self.lock().link_by_index_mut(index) {
            link.up = false;
        }
        Ok(())
    }

    async fn set_link_master(
        &self,
        slave_index: u32,
        master_index: u32,
    ) -> netlib::link::Result<()> {
        if let Some(link) = self.lock().link_by_index_mut(slave_index) {
            link.master_index = Some(master_index);
        }
        Ok(())
    }

    async fn delete_link(&self, index: u32) -> netlib::link::Result<()> {
        let mut s = self.lock();
        s.links.retain(|_, l| l.index != index);
        s.ipv4_addrs.remove(&index);
        s.ipv6_addrs.remove(&index);
        Ok(())
    }

    async fn probe_interfaces_for_carrier(
        &self,
        interfaces: &[(u32, &str)],
        _timeout: Duration,
    ) -> HashMap<u32, bool> {
        let s = self.lock();
        interfaces
            .iter()
            .map(|(idx, name)| (*idx, s.links.get(*name).is_some_and(|l| l.up)))
            .collect()
    }
}

impl AddressOps for MockNetlinkOps {
    async fn ensure_ipv4(
        &self,
        index: u32,
        ip: Ipv4Addr,
        prefix: u8,
    ) -> netlib::address::Result<()> {
        self.lock()
            .ipv4_addrs
            .entry(index)
            .or_default()
            .insert((ip, prefix));
        Ok(())
    }

    async fn find_ipv4(&self, index: u32) -> netlib::address::Result<Option<(Ipv4Addr, u8)>> {
        Ok(self
            .lock()
            .ipv4_addrs
            .get(&index)
            .and_then(|s| s.iter().next().copied()))
    }

    async fn has_ipv4(&self, index: u32) -> netlib::address::Result<bool> {
        Ok(self
            .lock()
            .ipv4_addrs
            .get(&index)
            .is_some_and(|s| !s.is_empty()))
    }

    async fn add_ipv4(&self, index: u32, ip: Ipv4Addr, prefix: u8) -> netlib::address::Result<()> {
        self.ensure_ipv4(index, ip, prefix).await
    }

    async fn remove_ipv4(&self, index: u32, ip: Ipv4Addr) -> netlib::address::Result<()> {
        if let Some(set) = self.lock().ipv4_addrs.get_mut(&index) {
            set.retain(|(addr, _)| *addr != ip);
        }
        Ok(())
    }

    async fn ensure_ipv6(
        &self,
        index: u32,
        ip: Ipv6Addr,
        prefix: u8,
    ) -> netlib::address::Result<()> {
        self.lock()
            .ipv6_addrs
            .entry(index)
            .or_default()
            .insert((ip, prefix));
        Ok(())
    }

    async fn remove_ipv6(&self, index: u32, ip: Ipv6Addr) -> netlib::address::Result<()> {
        if let Some(set) = self.lock().ipv6_addrs.get_mut(&index) {
            set.retain(|(addr, _)| *addr != ip);
        }
        Ok(())
    }
}

impl RouteOps for MockNetlinkOps {
    async fn ensure_default_route(&self, gateway: Ipv4Addr) -> netlib::route::Result<()> {
        self.lock().default_routes_v4.insert(gateway);
        Ok(())
    }

    async fn ensure_default_route_v6(&self, gateway: Ipv6Addr) -> netlib::route::Result<()> {
        self.lock().default_routes_v6.insert(gateway);
        Ok(())
    }

    async fn remove_default_route_v6(&self, gateway: Ipv6Addr) -> netlib::route::Result<()> {
        self.lock().default_routes_v6.remove(&gateway);
        Ok(())
    }
}

impl BridgeOps for MockNetlinkOps {
    async fn ensure_bridge(
        &self,
        bridge_name: &str,
        _physical_iface: &str,
        _gateway: Option<Ipv4Addr>,
        _stp: bool,
    ) -> netlib::bridge::Result<()> {
        let mut s = self.lock();
        let index = s.next_index;
        s.next_index += 1;
        s.links.insert(
            bridge_name.to_string(),
            MockLink {
                index,
                mac: [0xBE, 0xEF, 0x00, 0x00, 0x00, 0x01],
                up: true,
                master_index: None,
            },
        );
        Ok(())
    }

    async fn attach_to_bridge(
        &self,
        iface_name: &str,
        bridge_name: &str,
    ) -> netlib::bridge::Result<()> {
        let bridge_index = self
            .lock()
            .links
            .get(bridge_name)
            .map(|l| l.index)
            .unwrap_or(0);
        if let Some(link) = self.lock().links.get_mut(iface_name) {
            link.master_index = Some(bridge_index);
        }
        Ok(())
    }
}

impl InterfaceOps for MockNetlinkOps {
    async fn discover_ethernet(&self) -> netlib::interface::Result<Vec<Interface>> {
        let s = self.lock();
        let mut out = Vec::with_capacity(s.links.len());
        for (name, link) in &s.links {
            out.push(Interface::new(
                InterfaceName::new(name.clone())?,
                link.index,
                link.mac,
                link_state_kind(link.up),
            ));
        }
        Ok(out)
    }
}

impl NetlinkOps for MockNetlinkOps {}

fn make_config() -> Arc<config::NetworkConfig> {
    Arc::new(config::NetworkConfig::default())
}

fn make_snapshot(name: &str, index: u32, mac: [u8; 6]) -> InterfaceSnapshot {
    InterfaceSnapshot {
        name: InterfaceName::new(name).expect("valid name"),
        state: InterfaceState::Discovered,
        index,
        mac,
        link: LinkStateKind::Up,
        ip: None,
        lease: None,
        dhcp_state: None,
        ipv6: None,
    }
}

async fn wait_for_state(
    handle: &networkd::interface::InterfaceActorHandle,
    expected: InterfaceState,
) {
    let mut rx = handle.state_rx.clone();
    let timeout = Duration::from_secs(5);
    let result = tokio::time::timeout(timeout, async {
        while rx.borrow().state != expected {
            rx.changed().await.expect("actor dropped unexpectedly");
        }
    })
    .await;
    assert!(
        result.is_ok(),
        "timed out waiting for state {expected:?}, current: {:?}",
        handle.state_rx.borrow().state
    );
}

async fn wait_for_ipv6(handle: &networkd::interface::InterfaceActorHandle) {
    let mut rx = handle.state_rx.clone();
    let timeout = Duration::from_secs(5);
    let result = tokio::time::timeout(timeout, async {
        while rx.borrow().ipv6.is_none() {
            rx.changed().await.expect("actor dropped unexpectedly");
        }
    })
    .await;
    assert!(result.is_ok(), "timed out waiting for ipv6 config");
}
