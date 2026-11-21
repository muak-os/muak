//! Stateless address autoconfiguration method.

mod ra;
mod solicit;
mod state;
mod timers;

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv6Addr};
use core::time::Duration;

use ::slaac::icmpv6::types::RouterAdvertisement;

use self::state::{ManagedAddress, ManagedDns, ManagedRouter};
use super::Config;
use crate::provision::{AddressConfig, Command, Event, Intent, Method, State};
use crate::transport::icmpv6::Transport;
use crate::view::V6State;

const MAX_RTR_SOLICITATIONS: u32 = 3;
const RTR_SOLICITATION_INTERVAL: Duration = Duration::from_secs(4);

/// Stateless address autoconfiguration method.
pub struct Slaac<T: Transport> {
    config: Config,
    interface: String,
    mac: [u8; 6],
    socket: T,
    running: bool,
    solicited: bool,
    solicitation_attempt: u32,
    address: Option<ManagedAddress>,
    router: Option<ManagedRouter>,
    dns_servers: Vec<ManagedDns>,
    queue: VecDeque<Event>,
}

impl<T: Transport> Slaac<T> {
    /// Creates a SLAAC method around an already-open `ICMPv6` transport.
    #[must_use = "the method owns the provisioning state"]
    pub const fn new(config: Config, interface: String, mac: [u8; 6], socket: T) -> Self {
        Self {
            config,
            interface,
            mac,
            socket,
            running: false,
            solicited: false,
            solicitation_attempt: 0,
            address: None,
            router: None,
            dns_servers: Vec::new(),
            queue: VecDeque::new(),
        }
    }

    fn dns_fallback(&self) -> Vec<ManagedDns> {
        self.config
            .dns
            .iter()
            .map(|&server| ManagedDns::new(server, u64::from(u32::MAX)))
            .collect()
    }

    const fn resolicit(&mut self) {
        self.running = true;
        self.solicited = false;
        self.solicitation_attempt = 0;
    }

    const fn suspend(&mut self) {
        self.running = false;
    }

    fn stop(&mut self) {
        self.running = false;
        self.solicited = true;
        if let Some(address) = self.address.take() {
            let intent = deconfigure(address.address, address.prefix_len);
            self.queue.push_back(Event::Intent(intent));
        }
        if let Some(router) = self.router.take() {
            self.queue
                .push_back(Event::Intent(Intent::RemoveDefaultRoute(IpAddr::V6(
                    router.value,
                ))));
        }
        self.dns_servers.clear();
        self.queue.push_back(Event::State(State::Stopped));
    }
}

impl<T: Transport + Send> Method for Slaac<T> {
    type View = V6State;

    async fn next_event(&mut self) -> Event {
        self.advance().await
    }

    fn on_command(&mut self, command: Command) {
        match command {
            Command::Start | Command::Reconcile | Command::LinkUp => self.resolicit(),
            Command::LinkDown => self.suspend(),
            Command::Stop => self.stop(),
        }
    }

    fn view(&self) -> Option<Self::View> {
        let address = self.address.as_ref()?;
        Some(V6State::Slaac {
            addresses: vec![(address.address, address.prefix_len)],
            routers: self.router.iter().map(|router| router.value).collect(),
            dns: self.dns_servers.iter().map(|dns| dns.value).collect(),
        })
    }
}

impl<T: Transport + Send> Slaac<T> {
    async fn advance(&mut self) -> Event {
        if let Some(event) = self.queue.pop_front() {
            return event;
        }
        self.solicit_if_needed().await;
        if let Some(event) = self.queue.pop_front() {
            return event;
        }
        if !self.running {
            return core::future::pending::<Event>().await;
        }
        solicit::monitor(self).await
    }

    async fn solicit_if_needed(&mut self) {
        if self.running && !self.solicited {
            self.solicit().await;
        }
    }

    async fn solicit(&mut self) {
        if let Err(reason) = solicit::initial(self).await {
            self.queue.push_back(Event::Warning(reason));
        }
        self.solicited = true;
    }
}

pub(super) const fn configure(address: Ipv6Addr, prefix_len: u8) -> Intent {
    Intent::Configure(AddressConfig {
        address: IpAddr::V6(address),
        prefix_len,
    })
}

pub(super) const fn deconfigure(address: Ipv6Addr, prefix_len: u8) -> Intent {
    Intent::Deconfigure(AddressConfig {
        address: IpAddr::V6(address),
        prefix_len,
    })
}

fn resolve_dns<T: Transport>(method: &Slaac<T>, ra: &RouterAdvertisement) -> Vec<ManagedDns> {
    if ra.dns_servers.is_empty() {
        method.dns_fallback()
    } else {
        ra.dns_servers
            .iter()
            .map(|&server| ManagedDns::new(server, u64::from(ra.dns_lifetime)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use alloc::collections::VecDeque;
    use core::future::Future;
    use core::net::{IpAddr, Ipv6Addr};
    use core::time::Duration;
    use std::sync::Mutex;

    use tokio::time::timeout;

    use super::Slaac;
    use crate::provision::v6::Config;
    use crate::provision::{Command, Event, Intent, Method as _, State};
    use crate::transport::icmpv6::Transport;
    use crate::view::V6State;

    const MAC: [u8; 6] = [0x00, 0x1a, 0x2b, 0x3c, 0x4d, 0x5e];
    const ROUTER: Ipv6Addr = Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1);
    const PREFIX: Ipv6Addr = Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0, 0, 0, 0);
    const ADDRESS: Ipv6Addr = Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0x021a, 0x2bff, 0xfe3c, 0x4d5e);
    const DNS: Ipv6Addr = Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0, 0, 0, 0x53);
    const ND_OPT_PREFIX_INFO: u8 = 3;
    const ND_OPT_RDNSS: u8 = 25;

    #[derive(Default)]
    struct MockTransport {
        inbound: Mutex<VecDeque<(Vec<u8>, Option<Ipv6Addr>)>>,
        sent: Mutex<Vec<Ipv6Addr>>,
    }

    impl MockTransport {
        fn push_ra(&self, packet: Vec<u8>, source: Ipv6Addr) {
            self.inbound
                .lock()
                .expect("inbound lock")
                .push_back((packet, Some(source)));
        }

        fn destinations(&self) -> Vec<Ipv6Addr> {
            self.sent.lock().expect("sent lock").clone()
        }

        fn pop_inbound(&self, buf: &mut [u8]) -> Option<(usize, Option<Ipv6Addr>)> {
            let (packet, source) = self.inbound.lock().expect("inbound lock").pop_front()?;
            let len = packet.len().min(buf.len());
            let source_bytes = packet.get(..len)?;
            buf.get_mut(..len)?.copy_from_slice(source_bytes);
            Some((len, source))
        }
    }

    impl Transport for MockTransport {
        async fn recv_from(&self, buf: &mut [u8]) -> std::io::Result<(usize, Option<Ipv6Addr>)> {
            match self.pop_inbound(buf) {
                Some(received) => Ok(received),
                None => core::future::pending().await,
            }
        }

        fn send_to(
            &self,
            dst: Ipv6Addr,
            payload: &[u8],
        ) -> impl Future<Output = std::io::Result<usize>> + Send {
            self.sent.lock().expect("sent lock").push(dst);
            core::future::ready(Ok(payload.len()))
        }
    }

    fn new_method(transport: MockTransport) -> Slaac<MockTransport> {
        let config = Config {
            addresses: Vec::new(),
            gateway: None,
            dns: Vec::new(),
            autoconf: true,
        };
        Slaac::new(config, String::from("eth0"), MAC, transport)
    }

    fn ra(router_lifetime: u16, prefix: Option<(u32, u32)>, dns: &[(Ipv6Addr, u32)]) -> Vec<u8> {
        let mut packet = Vec::new();
        packet.extend([::slaac::icmpv6::ROUTER_ADVERTISEMENT, 0, 0, 0, 64, 0]);
        packet.extend(router_lifetime.to_be_bytes());
        packet.extend([0_u8; 8]);
        if let Some((valid, preferred)) = prefix {
            packet.extend([ND_OPT_PREFIX_INFO, 4, 64, 0x40]);
            packet.extend(valid.to_be_bytes());
            packet.extend(preferred.to_be_bytes());
            packet.extend([0_u8; 4]);
            packet.extend(PREFIX.octets());
        }
        if let Some(&(server, lifetime)) = dns.first() {
            packet.extend([ND_OPT_RDNSS, 3, 0, 0]);
            packet.extend(lifetime.to_be_bytes());
            packet.extend(server.octets());
        }
        packet
    }

    #[tokio::test(start_paused = true)]
    async fn missing_router_advertisement_warns() {
        // ARRANGE
        let transport = MockTransport::default();
        let mut method = new_method(transport);

        // ACT
        method.on_command(Command::Start);
        let event = method.next_event().await;

        // ASSERT
        assert!(matches!(event, Event::Warning(_)));
        assert!(method.view().is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn solicitation_sends_rs_and_configures_address() {
        // ARRANGE
        let transport = MockTransport::default();
        transport.push_ra(ra(1800, Some((7200, 3600)), &[]), ROUTER);
        let mut method = new_method(transport);

        // ACT
        method.on_command(Command::Start);
        let configure = method.next_event().await;
        let route = method.next_event().await;
        let bound = method.next_event().await;

        // ASSERT
        assert_eq!(
            method.socket.destinations(),
            vec![::slaac::icmpv6::ALL_ROUTERS]
        );
        assert!(matches!(
            configure,
            Event::Intent(Intent::Configure(config))
                if config.address == IpAddr::V6(ADDRESS) && config.prefix_len == 64
        ));
        assert!(matches!(
            route,
            Event::Intent(Intent::EnsureDefaultRoute(IpAddr::V6(ROUTER)))
        ));
        assert!(matches!(bound, Event::State(State::Bound)));
    }

    #[tokio::test(start_paused = true)]
    async fn preferred_lifetime_expiry_keeps_address_deprecated() {
        // ARRANGE
        let transport = MockTransport::default();
        transport.push_ra(ra(1800, Some((10, 1)), &[]), ROUTER);
        let mut method = new_method(transport);
        method.on_command(Command::Start);
        let _configure = method.next_event().await;
        let _route = method.next_event().await;
        let _bound = method.next_event().await;

        // ACT
        let outcome = timeout(Duration::from_secs(2), method.next_event()).await;

        // ASSERT
        let _elapsed = outcome.expect_err("deprecation must not deconfigure the address");
        assert!(matches!(
            method.view(),
            Some(V6State::Slaac { addresses, .. }) if addresses == [(ADDRESS, 64)]
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn router_expiry_removes_default_route() {
        // ARRANGE
        let transport = MockTransport::default();
        transport.push_ra(ra(1, Some((100, 100)), &[]), ROUTER);
        let mut method = new_method(transport);
        method.on_command(Command::Start);
        let _configure = method.next_event().await;
        let _route = method.next_event().await;
        let _bound = method.next_event().await;

        // ACT
        let expired = method.next_event().await;

        // ASSERT
        assert!(matches!(
            expired,
            Event::Intent(Intent::RemoveDefaultRoute(IpAddr::V6(ROUTER)))
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn rdnss_update_is_reflected_in_view() {
        // ARRANGE
        let transport = MockTransport::default();
        transport.push_ra(ra(1800, Some((100, 100)), &[]), ROUTER);
        let mut method = new_method(transport);
        method.on_command(Command::Start);
        let _configure = method.next_event().await;
        let _route = method.next_event().await;
        let _bound = method.next_event().await;
        assert!(matches!(
            method.view(),
            Some(V6State::Slaac { dns, .. }) if dns.is_empty()
        ));
        method
            .socket
            .push_ra(ra(1800, Some((100, 100)), &[(DNS, 3600)]), ROUTER);

        // ACT
        let _outcome = timeout(Duration::from_millis(10), method.next_event()).await;

        // ASSERT
        assert!(matches!(
            method.view(),
            Some(V6State::Slaac { dns, .. }) if dns == vec![DNS]
        ));
    }
}
