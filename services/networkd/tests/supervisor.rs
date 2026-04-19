//! Integration tests for the network supervisor.

#[path = "supervisor/discovery.rs"]
mod discovery;
#[path = "supervisor/dispatch.rs"]
mod dispatch;
#[path = "supervisor/dns.rs"]
mod dns;
#[path = "supervisor/failover.rs"]
mod failover;
#[path = "supervisor/lifecycle.rs"]
mod lifecycle;
#[path = "supervisor/provision.rs"]
mod provision;
#[path = "supervisor/reconcile.rs"]
mod reconcile;

use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use netlib::interface::{Interface, InterfaceName};
use netlib::link::LinkStateKind;
use netlib::ops::{AddressOps, BridgeOps, InterfaceOps, LinkOps, NetlinkOps, RouteOps};
use networkd::supervisor;

fn make_config() -> Arc<config::NetworkConfig> {
    let mut cfg = config::NetworkConfig::default();
    cfg.dns.clear();
    cfg.interfaces.clear();
    cfg.interfaces.push(config::InterfaceConfig {
        name: "auto".to_string(),
        kind: config::InterfaceKind::Ethernet,
        ipv4: Some(config::Ipv4InterfaceConfig {
            dhcp: false,
            addresses: vec![config::Cidr4 {
                address: std::net::Ipv4Addr::new(10, 0, 0, 2),
                prefix: 24,
            }],
            gateway: Some(std::net::Ipv4Addr::new(10, 0, 0, 1)),
        }),
        ipv6: None,
        bridge: None,
    });
    Arc::new(cfg)
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

#[derive(Debug, Clone)]
struct MockLink {
    index: u32,
    mac: [u8; 6],
    up: bool,
}

#[derive(Clone, Debug)]
struct MockNetlinkOps {
    state: Arc<Mutex<MockInner>>,
}

impl MockNetlinkOps {
    fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(MockInner {
                next_index: 1,
                ..MockInner::default()
            })),
        }
    }

    fn add_link(&self, name: &str, mac: [u8; 6], up: bool) -> u32 {
        let mut s = self.state.lock().expect("mock lock poisoned");
        let index = s.next_index;
        s.next_index += 1;
        s.links
            .insert(name.to_string(), MockLink { index, mac, up });
        index
    }

    fn ipv4_addrs(&self, index: u32) -> HashSet<(Ipv4Addr, u8)> {
        self.state
            .lock()
            .expect("lock")
            .ipv4_addrs
            .get(&index)
            .cloned()
            .unwrap_or_default()
    }

    fn ipv6_addrs(&self, index: u32) -> HashSet<(Ipv6Addr, u8)> {
        self.state
            .lock()
            .expect("lock")
            .ipv6_addrs
            .get(&index)
            .cloned()
            .unwrap_or_default()
    }

    fn has_link(&self, name: &str) -> bool {
        self.state.lock().expect("lock").links.contains_key(name)
    }

    fn has_default_route_v6(&self, gateway: Ipv6Addr) -> bool {
        self.state
            .lock()
            .expect("lock")
            .default_routes_v6
            .contains(&gateway)
    }
}

impl LinkOps for MockNetlinkOps {
    async fn link_exists(&self, name: &str) -> netlib::link::Result<bool> {
        Ok(self.state.lock().expect("lock").links.contains_key(name))
    }

    async fn get_link_index(&self, name: &str) -> netlib::link::Result<u32> {
        self.state
            .lock()
            .expect("lock")
            .links
            .get(name)
            .map(|l| l.index)
            .ok_or_else(|| netlib::link::Error::NotFound(name.to_string()))
    }

    async fn ensure_link_up(&self, name: &str) -> netlib::link::Result<u32> {
        let mut s = self.state.lock().expect("lock");
        let link = s
            .links
            .get_mut(name)
            .ok_or_else(|| netlib::link::Error::NotFound(name.to_string()))?;
        link.up = true;
        Ok(link.index)
    }

    async fn bring_link_up(&self, index: u32) -> netlib::link::Result<()> {
        let mut s = self.state.lock().expect("lock");
        if let Some(l) = s.links.values_mut().find(|l| l.index == index) {
            l.up = true;
        }
        Ok(())
    }

    async fn bring_link_down(&self, index: u32) -> netlib::link::Result<()> {
        let mut s = self.state.lock().expect("lock");
        if let Some(l) = s.links.values_mut().find(|l| l.index == index) {
            l.up = false;
        }
        Ok(())
    }

    async fn set_link_master(&self, _slave: u32, _master: u32) -> netlib::link::Result<()> {
        Ok(())
    }

    async fn delete_link(&self, index: u32) -> netlib::link::Result<()> {
        self.state
            .lock()
            .expect("lock")
            .links
            .retain(|_, l| l.index != index);
        Ok(())
    }

    async fn probe_interfaces_for_carrier(
        &self,
        interfaces: &[(u32, &str)],
        _timeout: Duration,
    ) -> HashMap<u32, bool> {
        let s = self.state.lock().expect("lock");
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
        ip: std::net::Ipv4Addr,
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
    async fn find_ipv4(
        &self,
        index: u32,
    ) -> netlib::address::Result<Option<(std::net::Ipv4Addr, u8)>> {
        Ok(self
            .state
            .lock()
            .expect("lock")
            .ipv4_addrs
            .get(&index)
            .and_then(|s| s.iter().next().copied()))
    }
    async fn has_ipv4(&self, index: u32) -> netlib::address::Result<bool> {
        Ok(self
            .state
            .lock()
            .expect("lock")
            .ipv4_addrs
            .get(&index)
            .is_some_and(|s| !s.is_empty()))
    }
    async fn add_ipv4(
        &self,
        index: u32,
        ip: std::net::Ipv4Addr,
        prefix: u8,
    ) -> netlib::address::Result<()> {
        self.ensure_ipv4(index, ip, prefix).await
    }
    async fn remove_ipv4(&self, index: u32, ip: std::net::Ipv4Addr) -> netlib::address::Result<()> {
        if let Some(set) = self.state.lock().expect("lock").ipv4_addrs.get_mut(&index) {
            set.retain(|(addr, _)| *addr != ip);
        }
        Ok(())
    }
    async fn ensure_ipv6(
        &self,
        index: u32,
        ip: std::net::Ipv6Addr,
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
    async fn remove_ipv6(&self, index: u32, ip: std::net::Ipv6Addr) -> netlib::address::Result<()> {
        if let Some(set) = self.state.lock().expect("lock").ipv6_addrs.get_mut(&index) {
            set.retain(|(addr, _)| *addr != ip);
        }
        Ok(())
    }
}

impl RouteOps for MockNetlinkOps {
    async fn ensure_default_route(&self, gateway: std::net::Ipv4Addr) -> netlib::route::Result<()> {
        self.state
            .lock()
            .expect("lock")
            .default_routes_v4
            .insert(gateway);
        Ok(())
    }
    async fn ensure_default_route_v6(
        &self,
        gateway: std::net::Ipv6Addr,
    ) -> netlib::route::Result<()> {
        self.state
            .lock()
            .expect("lock")
            .default_routes_v6
            .insert(gateway);
        Ok(())
    }
    async fn remove_default_route_v6(
        &self,
        gateway: std::net::Ipv6Addr,
    ) -> netlib::route::Result<()> {
        self.state
            .lock()
            .expect("lock")
            .default_routes_v6
            .remove(&gateway);
        Ok(())
    }
}

impl BridgeOps for MockNetlinkOps {
    async fn ensure_bridge(
        &self,
        bridge: &str,
        _: &str,
        _: Option<std::net::Ipv4Addr>,
        _: bool,
    ) -> netlib::bridge::Result<()> {
        let mut s = self.state.lock().expect("lock");
        let index = s.next_index;
        s.next_index += 1;
        s.links.insert(
            bridge.to_string(),
            MockLink {
                index,
                mac: [0; 6],
                up: true,
            },
        );
        Ok(())
    }

    async fn attach_to_bridge(&self, _: &str, _: &str) -> netlib::bridge::Result<()> {
        Ok(())
    }
}

impl InterfaceOps for MockNetlinkOps {
    async fn discover_ethernet(&self) -> netlib::interface::Result<Vec<Interface>> {
        let s = self.state.lock().expect("lock");
        let mut out = Vec::with_capacity(s.links.len());
        for (name, link) in &s.links {
            let kind = if link.up {
                LinkStateKind::Up
            } else {
                LinkStateKind::Down
            };
            out.push(Interface::new(
                InterfaceName::new(name.clone())?,
                link.index,
                link.mac,
                kind,
            ));
        }
        Ok(out)
    }
}

impl NetlinkOps for MockNetlinkOps {}
