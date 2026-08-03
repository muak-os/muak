//! Integration tests for the per-interface actor.

extern crate alloc;

#[cfg(test)]
mod interface {
    pub(super) use super::*;

    mod actor;
    mod bridge;
    mod dhcp;
    mod link;
    mod reconcile;
    mod slaac;
    mod r#static;
}

use alloc::sync::Arc;
use core::net::{Ipv4Addr, Ipv6Addr};
use core::time::Duration;
use std::collections::{HashMap, HashSet};
use std::os::fd::{FromRawFd as _, IntoRawFd as _, OwnedFd};
use std::sync::{Mutex, MutexGuard};

use anyhow::Result;
use netlib::address;
use netlib::bridge;
use netlib::interface as net_iface;
use netlib::interface::{Ethernet, Name};
use netlib::link;
use netlib::link::{Failure, State};
use netlib::netlink;
use netlib::packet::Socket;
use netlib::route;
use networkd::dhcp::client::DhcpConnector;
use networkd::interface::Actor;
use networkd::interface::ActorHandle;
use networkd::interface::commands::Command;
use networkd::interface::snapshot::Snapshot;
use networkd::interface::state::Lifecycle;
use tokio::net::UdpSocket;
use tokio::time::timeout;

#[derive(Clone, Default)]
struct MockDhcpConnector;

impl DhcpConnector for MockDhcpConnector {
    async fn create_raw(&self, _interface: &str) -> Result<Socket> {
        let udp = UdpSocket::bind("127.0.0.1:0").await?;
        let std_udp = udp.into_std()?;
        std_udp.set_nonblocking(true)?;
        let raw = std_udp.into_raw_fd();
        // SAFETY: raw is an owned fd from std::net::UdpSocket
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        Ok(Socket::from_fd(fd, 0)?)
    }

    async fn create_unicast(
        &self,
        _interface: &str,
        _src_ip: core::net::Ipv4Addr,
    ) -> Result<UdpSocket> {
        Ok(UdpSocket::bind("127.0.0.1:0").await?)
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
        self.links.values_mut().find(|link| link.index == index)
    }
}

#[derive(Debug, Clone)]
struct MockLink {
    index: u32,
    mac: [u8; 6],
    up: bool,
    master_index: Option<u32>,
}

/// In-memory implementation of `netlib::netlink::Ops` for deterministic testing.
#[derive(Clone, Debug)]
pub struct MockNetlinkOps {
    state: Arc<Mutex<MockInner>>,
}

impl Default for MockNetlinkOps {
    fn default() -> Self {
        Self::new()
    }
}

impl MockNetlinkOps {
    /// Creates an empty mock with no links.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(MockInner {
                next_index: 1,
                ..MockInner::default()
            })),
        }
    }

    fn lock(&self) -> MutexGuard<'_, MockInner> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Adds a pre-existing link so `discover_ethernet` will return it.
    #[must_use]
    pub fn add_link(&self, name: &str, mac: [u8; 6], up: bool) -> u32 {
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

    /// Returns a snapshot of IPv4 addresses for an index.
    #[must_use]
    pub fn ipv4_addrs(&self, index: u32) -> HashSet<(Ipv4Addr, u8)> {
        self.lock()
            .ipv4_addrs
            .get(&index)
            .cloned()
            .unwrap_or_default()
    }

    /// Returns a snapshot of IPv6 addresses for an index.
    #[must_use]
    pub fn ipv6_addrs(&self, index: u32) -> HashSet<(Ipv6Addr, u8)> {
        self.lock()
            .ipv6_addrs
            .get(&index)
            .cloned()
            .unwrap_or_default()
    }

    /// Returns true if the given IPv4 gateway is in the default route table.
    #[must_use]
    pub fn has_default_route_v4(&self, gw: Ipv4Addr) -> bool {
        self.lock().default_routes_v4.contains(&gw)
    }

    /// Returns true if the given IPv6 gateway is in the default route table.
    #[must_use]
    pub fn has_default_route_v6(&self, gw: Ipv6Addr) -> bool {
        self.lock().default_routes_v6.contains(&gw)
    }
}

fn link_state_kind(up: bool) -> State {
    if up { State::Up } else { State::Down }
}

impl link::Ops for MockNetlinkOps {
    async fn exists(&self, name: &str) -> link::Result<bool> {
        Ok(self.lock().links.contains_key(name))
    }

    async fn index(&self, name: &str) -> link::Result<u32> {
        self.lock()
            .links
            .get(name)
            .map(|link| link.index)
            .ok_or_else(|| Failure::NotFound(name.to_owned()))
    }

    async fn ensure_up(&self, name: &str) -> link::Result<u32> {
        let mut state = self.lock();
        let link = state
            .links
            .get_mut(name)
            .ok_or_else(|| Failure::NotFound(name.to_owned()))?;
        link.up = true;
        Ok(link.index)
    }

    async fn bring_up(&self, index: u32) -> link::Result<()> {
        if let Some(link) = self.lock().link_by_index_mut(index) {
            link.up = true;
        }
        Ok(())
    }

    async fn bring_down(&self, index: u32) -> link::Result<()> {
        if let Some(link) = self.lock().link_by_index_mut(index) {
            link.up = false;
        }
        Ok(())
    }

    async fn set_master(&self, slave_index: u32, master_index: u32) -> link::Result<()> {
        if let Some(link) = self.lock().link_by_index_mut(slave_index) {
            link.master_index = Some(master_index);
        }
        Ok(())
    }

    async fn delete(&self, index: u32) -> link::Result<()> {
        let mut state = self.lock();
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
        let state = self.lock();
        interfaces
            .iter()
            .map(|&(idx, name)| (idx, state.links.get(name).is_some_and(|link| link.up)))
            .collect()
    }
}

impl address::Ops for MockNetlinkOps {
    async fn ensure_ipv4(&self, index: u32, ip: Ipv4Addr, prefix: u8) -> address::Result<()> {
        self.lock()
            .ipv4_addrs
            .entry(index)
            .or_default()
            .insert((ip, prefix));
        Ok(())
    }

    async fn find_ipv4(&self, index: u32) -> address::Result<Option<(Ipv4Addr, u8)>> {
        Ok(self
            .lock()
            .ipv4_addrs
            .get(&index)
            .and_then(|addrs| addrs.iter().next().copied()))
    }

    async fn has_ipv4(&self, index: u32) -> address::Result<bool> {
        Ok(self
            .lock()
            .ipv4_addrs
            .get(&index)
            .is_some_and(|addrs| !addrs.is_empty()))
    }

    async fn add_ipv4(&self, index: u32, ip: Ipv4Addr, prefix: u8) -> address::Result<()> {
        self.ensure_ipv4(index, ip, prefix).await
    }

    async fn remove_ipv4(&self, index: u32, ip: Ipv4Addr) -> address::Result<()> {
        if let Some(set) = self.lock().ipv4_addrs.get_mut(&index) {
            set.retain(|&(addr, _)| addr != ip);
        }
        Ok(())
    }

    async fn ensure_ipv6(&self, index: u32, ip: Ipv6Addr, prefix: u8) -> address::Result<()> {
        self.lock()
            .ipv6_addrs
            .entry(index)
            .or_default()
            .insert((ip, prefix));
        Ok(())
    }

    async fn remove_ipv6(&self, index: u32, ip: Ipv6Addr) -> address::Result<()> {
        if let Some(set) = self.lock().ipv6_addrs.get_mut(&index) {
            set.retain(|&(addr, _)| addr != ip);
        }
        Ok(())
    }
}

impl route::Ops for MockNetlinkOps {
    async fn ensure_default_route(&self, gateway: Ipv4Addr) -> route::Result<()> {
        self.lock().default_routes_v4.insert(gateway);
        Ok(())
    }

    async fn ensure_default_route_v6(&self, gateway: Ipv6Addr) -> route::Result<()> {
        self.lock().default_routes_v6.insert(gateway);
        Ok(())
    }

    async fn remove_default_route_v6(&self, gateway: Ipv6Addr) -> route::Result<()> {
        self.lock().default_routes_v6.remove(&gateway);
        Ok(())
    }
}

impl bridge::Ops for MockNetlinkOps {
    async fn ensure_bridge(
        &self,
        bridge_name: &str,
        _physical_iface: &str,
        _gateway: Option<Ipv4Addr>,
        _stp: bool,
    ) -> bridge::Result<()> {
        let mut state = self.lock();
        let index = state.next_index;
        state.next_index = state.next_index.saturating_add(1);
        state.links.insert(
            bridge_name.to_owned(),
            MockLink {
                index,
                mac: [0xBE, 0xEF, 0x00, 0x00, 0x00, 0x01],
                up: true,
                master_index: None,
            },
        );
        Ok(())
    }

    async fn attach_to_bridge(&self, iface_name: &str, bridge_name: &str) -> bridge::Result<()> {
        let bridge_index = self
            .lock()
            .links
            .get(bridge_name)
            .map_or(0, |link| link.index);
        if let Some(link) = self.lock().links.get_mut(iface_name) {
            link.master_index = Some(bridge_index);
        }
        Ok(())
    }
}

impl net_iface::Ops for MockNetlinkOps {
    async fn discover_ethernet(&self) -> net_iface::Result<Vec<Ethernet>> {
        let state = self.lock();
        let mut links: Vec<_> = state.links.iter().collect();
        links.sort_by(|left, right| left.0.cmp(right.0));
        let mut out = Vec::with_capacity(links.len());
        for (name, link) in links {
            out.push(Ethernet::new(
                Name::new(name.clone())?,
                link.index,
                link.mac,
                link_state_kind(link.up),
            ));
        }
        Ok(out)
    }
}

impl netlink::Ops for MockNetlinkOps {}

fn make_config() -> Arc<config::NetworkConfig> {
    Arc::new(config::NetworkConfig::default())
}

fn make_snapshot(name: Name, index: u32, mac: [u8; 6]) -> Snapshot {
    Snapshot {
        name: name.clone(),
        state: Lifecycle::Discovered,
        index,
        mac,
        link: State::Up,
        ip: None,
        lease: None,
        dhcp_state: None,
        ipv6: None,
        l3_owner: name,
    }
}

async fn wait_for_state(handle: &ActorHandle, expected: Lifecycle) {
    let mut rx = handle.state_rx.clone();
    let timeout_duration = Duration::from_secs(5);
    let reached = timeout(timeout_duration, async {
        while rx.borrow().state != expected {
            if rx.changed().await.is_err() {
                return false;
            }
        }
        true
    })
    .await
    .unwrap_or(false);
    assert!(
        reached,
        "timed out waiting for state {expected:?}, current: {:?}",
        handle.state_rx.borrow().state
    );
}

async fn wait_for_ipv6(handle: &ActorHandle) {
    let mut rx = handle.state_rx.clone();
    let timeout_duration = Duration::from_secs(5);
    let reached = timeout(timeout_duration, async {
        while rx.borrow().ipv6.is_none() {
            if rx.changed().await.is_err() {
                return false;
            }
        }
        true
    })
    .await
    .unwrap_or(false);
    assert!(reached, "timed out waiting for ipv6 config");
}
