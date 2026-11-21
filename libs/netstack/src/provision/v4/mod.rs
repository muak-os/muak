//! IPv4 address provisioning methods.

mod dhcp;
mod intent;
mod lease;
mod timer;

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;
use core::future::Future;
use core::net::{IpAddr, Ipv4Addr};

use dhcp::Phase;
use dhcp_client::lease::Lease;
use timer::Deadlines;

use crate::provision::{AddressConfig, Command, Event, Intent, Method, State};
use crate::view::V4State;

/// Static IPv4 address configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// Address and prefix length pairs.
    pub addresses: Vec<(Ipv4Addr, u8)>,
    /// Optional default gateway.
    pub gateway: Option<Ipv4Addr>,
    /// DNS server addresses.
    pub dns: Vec<Ipv4Addr>,
    /// Whether the interface acquires its address over `DHCPv4`.
    pub dhcp: bool,
}

/// `DHCPv4` exchange operations used by [`Dhcp`].
pub trait Exchange: Send + Sync {
    /// Runs a full `DORA` exchange and returns the granted lease.
    fn acquire(
        &self,
        interface: &str,
        mac: [u8; 6],
    ) -> impl Future<Output = dhcp_client::Result<Lease>> + Send;

    /// Rebinds the lease by broadcasting to any server that answers.
    fn rebind(
        &self,
        interface: &str,
        mac: [u8; 6],
        server: Ipv4Addr,
        assigned: Ipv4Addr,
    ) -> impl Future<Output = dhcp_client::Result<Lease>> + Send;

    /// Renews the lease directly with the granting server.
    fn renew(
        &self,
        interface: &str,
        mac: [u8; 6],
        server: Ipv4Addr,
        assigned: Ipv4Addr,
    ) -> impl Future<Output = dhcp_client::Result<Lease>> + Send;
}

/// `DHCPv4` address method driven by lease timers.
pub struct Dhcp<E: Exchange> {
    exchange: E,
    interface: String,
    mac: [u8; 6],
    phase: Phase,
    lease: Option<Lease>,
    deadlines: Deadlines,
    queue: VecDeque<Event>,
}

/// Static IPv4 address method.
pub struct Static {
    config: Config,
    started: bool,
    queue: VecDeque<Event>,
}

impl Static {
    /// Creates a static IPv4 method from the given configuration.
    #[must_use = "the method owns the provisioning state"]
    pub const fn new(config: Config) -> Self {
        Self {
            config,
            started: false,
            queue: VecDeque::new(),
        }
    }

    fn provision(&mut self) {
        self.started = true;
        for &(address, prefix_len) in &self.config.addresses {
            let intent = Self::configure(address, prefix_len);
            self.queue.push_back(Event::Intent(intent));
        }
        if let Some(gateway) = self.config.gateway {
            self.queue
                .push_back(Event::Intent(Intent::EnsureDefaultRoute(IpAddr::V4(
                    gateway,
                ))));
        }
        self.queue.push_back(Event::State(State::Bound));
    }

    fn stop(&mut self) {
        self.started = false;
        for &(address, prefix_len) in &self.config.addresses {
            let intent = Self::deconfigure(address, prefix_len);
            self.queue.push_back(Event::Intent(intent));
        }
        if let Some(gateway) = self.config.gateway {
            self.queue
                .push_back(Event::Intent(Intent::RemoveDefaultRoute(IpAddr::V4(
                    gateway,
                ))));
        }
        self.queue.push_back(Event::State(State::Stopped));
    }

    const fn configure(address: Ipv4Addr, prefix_len: u8) -> Intent {
        Intent::Configure(AddressConfig {
            address: IpAddr::V4(address),
            prefix_len,
        })
    }

    const fn deconfigure(address: Ipv4Addr, prefix_len: u8) -> Intent {
        Intent::Deconfigure(AddressConfig {
            address: IpAddr::V4(address),
            prefix_len,
        })
    }
}

impl Method for Static {
    type View = V4State;

    async fn next_event(&mut self) -> Event {
        if let Some(event) = self.queue.pop_front() {
            return event;
        }
        core::future::pending::<Event>().await
    }

    fn on_command(&mut self, command: Command) {
        match command {
            Command::Start | Command::Reconcile | Command::LinkUp => self.provision(),
            Command::LinkDown => self.queue.push_back(Event::State(State::Degraded)),
            Command::Stop => self.stop(),
        }
    }

    fn view(&self) -> Option<Self::View> {
        if !self.started {
            return None;
        }
        let &(address, prefix_len) = self.config.addresses.first()?;
        Some(V4State::Static(netlink::address::IpConfig {
            address,
            prefix_len,
            gateway: self.config.gateway,
            dns: self.config.dns.clone(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use core::net::{IpAddr, Ipv4Addr};

    use super::{Config, Static};
    use crate::provision::{Command, Event, Intent, Method as _, State};
    use crate::view::V4State;

    const ADDRESS_A: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 2);
    const ADDRESS_B: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 3);
    const GATEWAY: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);
    const DNS: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 53);

    fn config() -> Config {
        Config {
            addresses: vec![(ADDRESS_A, 24), (ADDRESS_B, 24)],
            gateway: Some(GATEWAY),
            dns: vec![DNS],
            dhcp: false,
        }
    }

    async fn expect_intent(method: &mut Static) -> Intent {
        match method.next_event().await {
            Event::Intent(intent) => intent,
            Event::State(_) | Event::Failed(_) => panic!("expected intent"),
        }
    }

    async fn drain_start_events(method: &mut Static) {
        for _ in 0_usize..4_usize {
            method.next_event().await;
        }
    }

    #[tokio::test]
    async fn start_queues_configure_route_and_bound() {
        // ARRANGE
        let mut method = Static::new(config());

        // ACT
        method.on_command(Command::Start);
        let first = expect_intent(&mut method).await;
        let second = expect_intent(&mut method).await;
        let route = expect_intent(&mut method).await;
        let state = method.next_event().await;

        // ASSERT
        assert_configure(first, ADDRESS_A);
        assert_configure(second, ADDRESS_B);
        assert!(matches!(
            route,
            Intent::EnsureDefaultRoute(IpAddr::V4(GATEWAY))
        ));
        assert!(matches!(state, Event::State(State::Bound)));
    }

    #[tokio::test]
    async fn view_reports_static_state_after_start() {
        // ARRANGE
        let mut method = Static::new(config());
        method.on_command(Command::Start);

        // ACT
        let view = method.view();

        // ASSERT
        assert_static_view(view);
    }

    #[test]
    fn view_survives_link_down_and_clears_after_stop() {
        // ARRANGE
        let mut method = Static::new(config());
        method.on_command(Command::Start);

        // ACT
        method.on_command(Command::LinkDown);
        let after_link_down = method.view();
        method.on_command(Command::Stop);
        let after_stop = method.view();

        // ASSERT
        assert_static_view(after_link_down);
        assert!(after_stop.is_none());
    }

    #[tokio::test]
    async fn stop_queues_deconfigure_route_and_stopped() {
        // ARRANGE
        let mut method = Static::new(config());

        // ACT
        method.on_command(Command::Stop);
        let first = expect_intent(&mut method).await;
        let second = expect_intent(&mut method).await;
        let route = expect_intent(&mut method).await;
        let state = method.next_event().await;

        // ASSERT
        assert_deconfigure(first, ADDRESS_A);
        assert_deconfigure(second, ADDRESS_B);
        assert!(matches!(
            route,
            Intent::RemoveDefaultRoute(IpAddr::V4(GATEWAY))
        ));
        assert!(matches!(state, Event::State(State::Stopped)));
    }

    #[tokio::test]
    async fn link_up_requeues_configure() {
        // ARRANGE
        let mut method = Static::new(config());
        method.on_command(Command::Start);
        drain_start_events(&mut method).await;

        // ACT
        method.on_command(Command::LinkUp);
        let first = expect_intent(&mut method).await;

        // ASSERT
        assert_configure(first, ADDRESS_A);
    }

    fn assert_static_view(view: Option<V4State>) {
        let Some(V4State::Static(config)) = view else {
            panic!("expected static view");
        };
        assert_eq!(config.address, ADDRESS_A);
        assert_eq!(config.prefix_len, 24);
        assert_eq!(config.gateway, Some(GATEWAY));
        assert_eq!(config.dns, vec![DNS]);
    }

    fn assert_configure(intent: Intent, address: Ipv4Addr) {
        let Intent::Configure(config) = intent else {
            panic!("expected configure");
        };
        assert_eq!(config.address, IpAddr::V4(address));
        assert_eq!(config.prefix_len, 24);
    }

    fn assert_deconfigure(intent: Intent, address: Ipv4Addr) {
        let Intent::Deconfigure(config) = intent else {
            panic!("expected deconfigure");
        };
        assert_eq!(config.address, IpAddr::V4(address));
        assert_eq!(config.prefix_len, 24);
    }
}
