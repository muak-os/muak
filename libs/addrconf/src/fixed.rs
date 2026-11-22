//! Family-neutral static address provisioning state machine.

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::net::IpAddr;
use std::time::Instant;

use crate::{AddressConfig, Command, Event, Intent, Method, State};

/// Address configuration shared by the static methods.
#[derive(Debug, Clone)]
pub struct Config<A> {
    /// Address and prefix length pairs.
    pub addresses: Vec<(A, u8)>,
    /// Optional default gateway.
    pub gateway: Option<A>,
    /// DNS server addresses.
    pub dns: Vec<A>,
}

/// Static configuration a family exposes as its view.
#[derive(Debug, Clone)]
pub struct StaticView<A> {
    /// Primary address.
    pub address: A,
    /// Prefix length in bits.
    pub prefix_len: u8,
    /// Optional default gateway.
    pub gateway: Option<A>,
    /// DNS server addresses.
    pub dns: Vec<A>,
}

/// Address family adapter for [`Static`].
pub trait Family {
    /// Concrete address type for the family.
    type Address: Copy + Eq;

    /// Converts a family address into a family-neutral address.
    fn ip(address: Self::Address) -> IpAddr;
}

/// Static address method generic over the address family.
pub struct Static<F: Family> {
    config: Config<F::Address>,
    started: bool,
    queue: VecDeque<Event>,
}

impl<F: Family> Static<F> {
    /// Creates a static method from the given configuration.
    #[must_use = "the method owns the provisioning state"]
    pub const fn new(config: Config<F::Address>) -> Self {
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
                .push_back(Event::Intent(Intent::EnsureDefaultRoute(F::ip(gateway))));
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
                .push_back(Event::Intent(Intent::RemoveDefaultRoute(F::ip(gateway))));
        }
        self.queue.push_back(Event::State(State::Stopped));
    }

    fn configure(address: F::Address, prefix_len: u8) -> Intent {
        Intent::Configure(AddressConfig {
            address: F::ip(address),
            prefix_len,
        })
    }

    fn deconfigure(address: F::Address, prefix_len: u8) -> Intent {
        Intent::Deconfigure(AddressConfig {
            address: F::ip(address),
            prefix_len,
        })
    }
}

impl<F: Family> Method for Static<F>
where
    F::Address: Send,
{
    type View = StaticView<F::Address>;

    async fn next_event(&mut self, _now: Instant) -> Event {
        if let Some(event) = self.queue.pop_front() {
            return event;
        }
        core::future::pending::<Event>().await
    }

    fn next_wake(&self, _now: Instant) -> Option<Instant> {
        None
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
        Some(StaticView {
            address,
            prefix_len,
            gateway: self.config.gateway,
            dns: self.config.dns.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use core::net::{IpAddr, Ipv4Addr};
    use core::task::{Context, Poll, Waker};
    use std::time::Instant;

    use super::{Config, Family, Static, StaticView};
    use crate::{Command, Event, Intent, Method as _, State};

    const ADDRESS_A: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 2);
    const ADDRESS_B: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 3);
    const GATEWAY: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);
    const DNS: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 53);

    struct TestFamily;

    impl Family for TestFamily {
        type Address = Ipv4Addr;

        fn ip(address: Ipv4Addr) -> IpAddr {
            IpAddr::V4(address)
        }
    }

    fn config() -> Config<Ipv4Addr> {
        Config {
            addresses: vec![(ADDRESS_A, 24), (ADDRESS_B, 24)],
            gateway: Some(GATEWAY),
            dns: vec![DNS],
        }
    }

    fn next_event(method: &mut Static<TestFamily>) -> Event {
        let mut event = core::pin::pin!(method.next_event(Instant::now()));

        match event.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(event) => event,
            Poll::Pending => panic!("expected a queued event"),
        }
    }

    fn expect_intent(method: &mut Static<TestFamily>) -> Intent {
        match next_event(method) {
            Event::Intent(intent) => intent,
            Event::State(_) | Event::Warning(_) | Event::Failed(_) => panic!("expected intent"),
        }
    }

    fn drain_start_events(method: &mut Static<TestFamily>) {
        for _ in 0_usize..4_usize {
            drop(next_event(method));
        }
    }

    #[test]
    fn start_queues_configure_route_and_bound() {
        // ARRANGE
        let mut method = Static::<TestFamily>::new(config());

        // ACT
        method.on_command(Command::Start);
        let first = expect_intent(&mut method);
        let second = expect_intent(&mut method);
        let route = expect_intent(&mut method);
        let state = next_event(&mut method);

        // ASSERT
        assert_configure(first, ADDRESS_A);
        assert_configure(second, ADDRESS_B);
        assert!(matches!(
            route,
            Intent::EnsureDefaultRoute(IpAddr::V4(GATEWAY))
        ));
        assert!(matches!(state, Event::State(State::Bound)));
    }

    #[test]
    fn stop_queues_deconfigure_route_and_stopped() {
        // ARRANGE
        let mut method = Static::<TestFamily>::new(config());

        // ACT
        method.on_command(Command::Stop);
        let first = expect_intent(&mut method);
        let second = expect_intent(&mut method);
        let route = expect_intent(&mut method);
        let state = next_event(&mut method);

        // ASSERT
        assert_deconfigure(first, ADDRESS_A);
        assert_deconfigure(second, ADDRESS_B);
        assert!(matches!(
            route,
            Intent::RemoveDefaultRoute(IpAddr::V4(GATEWAY))
        ));
        assert!(matches!(state, Event::State(State::Stopped)));
        assert!(method.view().is_none());
    }

    #[test]
    fn start_without_addresses_binds_without_view() {
        // ARRANGE
        let mut method = Static::<TestFamily>::new(Config {
            addresses: Vec::new(),
            gateway: None,
            dns: Vec::new(),
        });

        // ACT
        method.on_command(Command::Start);
        let event = next_event(&mut method);
        let view = method.view();

        // ASSERT
        assert!(matches!(event, Event::State(State::Bound)));
        assert!(view.is_none());
    }

    #[test]
    fn link_up_requeues_configure() {
        // ARRANGE
        let mut method = Static::<TestFamily>::new(config());
        method.on_command(Command::Start);
        drain_start_events(&mut method);

        // ACT
        method.on_command(Command::LinkUp);
        let first = expect_intent(&mut method);

        // ASSERT
        assert_configure(first, ADDRESS_A);
    }

    #[test]
    fn link_down_degrades_and_keeps_view() {
        // ARRANGE
        let mut method = Static::<TestFamily>::new(config());
        method.on_command(Command::Start);
        drain_start_events(&mut method);

        // ACT
        method.on_command(Command::LinkDown);
        let event = next_event(&mut method);
        let view = method.view();

        // ASSERT
        assert!(matches!(event, Event::State(State::Degraded)));
        assert_view(view);
    }

    #[test]
    fn view_is_none_before_start() {
        // ARRANGE
        let method = Static::<TestFamily>::new(config());

        // ACT
        let view = method.view();

        // ASSERT
        assert!(view.is_none());
    }

    fn assert_view(view: Option<StaticView<Ipv4Addr>>) {
        let view = view.expect("view");
        assert_eq!(view.address, ADDRESS_A);
        assert_eq!(view.prefix_len, 24);
        assert_eq!(view.gateway, Some(GATEWAY));
        assert_eq!(view.dns, vec![DNS]);
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
