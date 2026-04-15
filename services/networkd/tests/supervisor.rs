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

use std::collections::HashMap;
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
        _: u32,
        _: std::net::Ipv4Addr,
        _: u8,
    ) -> netlib::address::Result<()> {
        Ok(())
    }
    async fn find_ipv4(&self, _: u32) -> netlib::address::Result<Option<(std::net::Ipv4Addr, u8)>> {
        Ok(None)
    }
    async fn has_ipv4(&self, _: u32) -> netlib::address::Result<bool> {
        Ok(false)
    }
    async fn add_ipv4(&self, _: u32, _: std::net::Ipv4Addr, _: u8) -> netlib::address::Result<()> {
        Ok(())
    }
    async fn remove_ipv4(&self, _: u32, _: std::net::Ipv4Addr) -> netlib::address::Result<()> {
        Ok(())
    }
    async fn ensure_ipv6(
        &self,
        _: u32,
        _: std::net::Ipv6Addr,
        _: u8,
    ) -> netlib::address::Result<()> {
        Ok(())
    }
    async fn remove_ipv6(&self, _: u32, _: std::net::Ipv6Addr) -> netlib::address::Result<()> {
        Ok(())
    }
}

impl RouteOps for MockNetlinkOps {
    async fn ensure_default_route(&self, _: std::net::Ipv4Addr) -> netlib::route::Result<()> {
        Ok(())
    }
    async fn ensure_default_route_v6(&self, _: std::net::Ipv6Addr) -> netlib::route::Result<()> {
        Ok(())
    }
    async fn remove_default_route_v6(&self, _: std::net::Ipv6Addr) -> netlib::route::Result<()> {
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
