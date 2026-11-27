//! Static address provisioning method.

use alloc::collections::VecDeque;
use core::net::IpAddr;
use std::time::{Instant, SystemTime};

use ip_method::{AddressConfig, Command, Effect, Event, Input, Intent, Method, Step};
use netmodel::MethodState;

use crate::{Config, Static, View};

impl<A: Copy + Into<IpAddr>> Static<A> {
    /// Creates a static method from the given configuration.
    #[must_use = "the method owns the provisioning state"]
    pub const fn new(config: Config<A>) -> Self {
        Self {
            config,
            started: false,
            queue: VecDeque::new(),
        }
    }

    /// Returns the configuration the method was created with.
    #[must_use = "callers need the configuration to report it"]
    pub const fn config(&self) -> &Config<A> {
        &self.config
    }

    fn apply(&mut self, command: Command) {
        match command {
            Command::Start | Command::Reconcile | Command::LinkUp => self.provision(),
            Command::LinkDown => self.queue.push_back(Event::State(MethodState::Degraded)),
            Command::Stop => self.stop(),
        }
    }

    fn provision(&mut self) {
        self.started = true;
        for &(address, prefix_len) in &self.config.addresses {
            let intent = Self::configure(address, prefix_len);
            self.queue.push_back(Event::Intent(intent));
        }
        if let Some(gateway) = self.config.gateway {
            let intent = Intent::EnsureDefaultRoute(gateway.into());
            self.queue.push_back(Event::Intent(intent));
        }
        self.queue.push_back(Event::State(MethodState::Bound));
    }

    fn stop(&mut self) {
        self.started = false;
        for &(address, prefix_len) in &self.config.addresses {
            let intent = Self::deconfigure(address, prefix_len);
            self.queue.push_back(Event::Intent(intent));
        }
        if let Some(gateway) = self.config.gateway {
            let intent = Intent::RemoveDefaultRoute(gateway.into());
            self.queue.push_back(Event::Intent(intent));
        }
        self.queue.push_back(Event::State(MethodState::Stopped));
    }

    fn configure(address: A, prefix_len: u8) -> Intent {
        Intent::Configure(AddressConfig {
            address: address.into(),
            prefix_len,
        })
    }

    fn deconfigure(address: A, prefix_len: u8) -> Intent {
        Intent::Deconfigure(AddressConfig {
            address: address.into(),
            prefix_len,
        })
    }
}

impl<A: Copy + Into<IpAddr>> Method for Static<A> {
    type View = View<A>;

    fn step(&mut self, _now: Instant, _wall: SystemTime, input: Input<'_>) -> Step<'_> {
        if let Input::Command(command) = input {
            self.apply(command);
        }

        match self.queue.pop_front() {
            Some(event) => Step::Effect(Effect::Emit(event)),
            None => Step::Wait,
        }
    }

    fn next_wake(&self, _now: Instant) -> Option<Instant> {
        None
    }

    fn view(&self) -> Option<Self::View> {
        if !self.started {
            return None;
        }
        let &(address, prefix_len) = self.config.addresses.first()?;

        Some(View {
            address,
            prefix_len,
            gateway: self.config.gateway,
            dns: self.config.dns.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use core::net::{IpAddr, Ipv4Addr};
    use std::time::{Instant, SystemTime};

    use ip_method::{Command, Effect, Event, Input, Intent, Method as _, Step};
    use netmodel::MethodState;

    use crate::{Config, Static, View};

    const ADDRESS_A: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 2);
    const ADDRESS_B: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 3);
    const GATEWAY: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);
    const DNS: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 53);

    fn config() -> Config<Ipv4Addr> {
        Config {
            addresses: vec![(ADDRESS_A, 24), (ADDRESS_B, 24)],
            gateway: Some(GATEWAY),
            dns: vec![DNS],
        }
    }

    fn send(method: &mut Static<Ipv4Addr>, input: Input<'_>) -> Option<Event> {
        let step = method.step(Instant::now(), SystemTime::UNIX_EPOCH, input);

        match step {
            Step::Effect(Effect::Emit(event)) => Some(event),
            Step::Effect(_) => panic!("expected an emitted event"),
            Step::Wait => None,
        }
    }

    fn command(method: &mut Static<Ipv4Addr>, command: Command) -> Event {
        send(method, Input::Command(command)).expect("command emits its first event")
    }

    fn next_event(method: &mut Static<Ipv4Addr>) -> Event {
        send(method, Input::Poll).expect("expected an emitted event")
    }

    fn intent_of(event: &Event) -> Intent {
        match event {
            &Event::Intent(intent) => intent,
            &Event::State(_) | &Event::Warning(_) | &Event::Failed(_) => panic!("expected intent"),
        }
    }

    fn expect_intent(method: &mut Static<Ipv4Addr>) -> Intent {
        intent_of(&next_event(method))
    }

    fn drain_events(method: &mut Static<Ipv4Addr>, count: usize) {
        for _ in 0_usize..count {
            drop(next_event(method));
        }
    }

    #[test]
    fn new_stores_config() {
        // ARRANGE
        let config = config();

        // ACT
        let method = Static::<Ipv4Addr>::new(config);

        // ASSERT
        assert_eq!(
            method.config().addresses,
            vec![(ADDRESS_A, 24), (ADDRESS_B, 24)]
        );
        assert_eq!(method.config().gateway, Some(GATEWAY));
        assert_eq!(method.config().dns, vec![DNS]);
    }

    #[test]
    fn start_queues_configure_route_and_bound() {
        // ARRANGE
        let mut method = Static::<Ipv4Addr>::new(config());

        // ACT
        let first = intent_of(&command(&mut method, Command::Start));
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
        assert!(matches!(state, Event::State(MethodState::Bound)));
        assert!(method.next_wake(Instant::now()).is_none());
    }

    #[test]
    fn stop_queues_deconfigure_route_and_stopped() {
        // ARRANGE
        let mut method = Static::<Ipv4Addr>::new(config());
        drop(command(&mut method, Command::Start));
        drain_events(&mut method, 3);

        // ACT
        let first = intent_of(&command(&mut method, Command::Stop));
        let second = expect_intent(&mut method);
        let route = expect_intent(&mut method);
        let state = next_event(&mut method);
        let view = method.view();

        // ASSERT
        assert_deconfigure(first, ADDRESS_A);
        assert_deconfigure(second, ADDRESS_B);
        assert!(matches!(
            route,
            Intent::RemoveDefaultRoute(IpAddr::V4(GATEWAY))
        ));
        assert!(matches!(state, Event::State(MethodState::Stopped)));
        assert!(view.is_none());
    }

    #[test]
    fn poll_waits_after_start_events_drained() {
        // ARRANGE
        let mut method = Static::<Ipv4Addr>::new(config());
        drop(command(&mut method, Command::Start));
        drain_events(&mut method, 3);

        // ACT
        let idle = send(&mut method, Input::Poll);

        // ASSERT
        assert!(idle.is_none());
    }

    #[test]
    fn poll_waits_after_stop_events_drained() {
        // ARRANGE
        let mut method = Static::<Ipv4Addr>::new(config());
        drop(command(&mut method, Command::Start));
        drain_events(&mut method, 3);
        drop(command(&mut method, Command::Stop));
        drain_events(&mut method, 3);

        // ACT
        let idle = send(&mut method, Input::Poll);

        // ASSERT
        assert!(idle.is_none());
    }

    #[test]
    fn reconcile_queues_configure_route_and_bound() {
        // ARRANGE
        let mut method = Static::<Ipv4Addr>::new(config());

        // ACT
        let first = intent_of(&command(&mut method, Command::Reconcile));
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
        assert!(matches!(state, Event::State(MethodState::Bound)));
    }

    #[test]
    fn stop_without_gateway_deconfigures_and_stops() {
        // ARRANGE
        let mut method = Static::<Ipv4Addr>::new(Config {
            addresses: vec![(ADDRESS_A, 24), (ADDRESS_B, 24)],
            gateway: None,
            dns: Vec::new(),
        });
        drop(command(&mut method, Command::Start));
        drain_events(&mut method, 2);

        // ACT
        let first = intent_of(&command(&mut method, Command::Stop));
        let second = expect_intent(&mut method);
        let state = next_event(&mut method);
        let idle = send(&mut method, Input::Poll);

        // ASSERT
        assert_deconfigure(first, ADDRESS_A);
        assert_deconfigure(second, ADDRESS_B);
        assert!(matches!(state, Event::State(MethodState::Stopped)));
        assert!(idle.is_none());
    }

    #[test]
    fn start_without_addresses_binds_without_view() {
        // ARRANGE
        let mut method = Static::<Ipv4Addr>::new(Config {
            addresses: Vec::new(),
            gateway: None,
            dns: Vec::new(),
        });

        // ACT
        let event = command(&mut method, Command::Start);
        let view = method.view();

        // ASSERT
        assert!(matches!(event, Event::State(MethodState::Bound)));
        assert!(view.is_none());
    }

    #[test]
    fn link_up_requeues_configure() {
        // ARRANGE
        let mut method = Static::<Ipv4Addr>::new(config());
        drop(command(&mut method, Command::Start));
        drain_events(&mut method, 3);

        // ACT
        let first = intent_of(&command(&mut method, Command::LinkUp));

        // ASSERT
        assert_configure(first, ADDRESS_A);
        assert_view(method.view());
    }

    #[test]
    fn link_down_degrades_and_keeps_view() {
        // ARRANGE
        let mut method = Static::<Ipv4Addr>::new(config());
        drop(command(&mut method, Command::Start));
        drain_events(&mut method, 3);

        // ACT
        let event = command(&mut method, Command::LinkDown);
        let view = method.view();

        // ASSERT
        assert!(matches!(event, Event::State(MethodState::Degraded)));
        assert_view(view);
    }

    #[test]
    fn view_is_none_before_start() {
        // ARRANGE
        let method = Static::<Ipv4Addr>::new(config());

        // ACT
        let view = method.view();

        // ASSERT
        assert!(view.is_none());
    }

    #[test]
    fn ignored_input_waits_when_idle() {
        // ARRANGE
        let mut method = Static::<Ipv4Addr>::new(config());

        // ACT
        let event = send(&mut method, Input::Entropy(7));

        // ASSERT
        assert!(event.is_none());
    }

    fn assert_view(view: Option<View<Ipv4Addr>>) {
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
