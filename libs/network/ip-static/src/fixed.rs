//! Static address provisioning method.

use core::net::IpAddr;
use std::time::{Instant, SystemTime};

use ip_method::{AddressConfig, Command, Effect, Event, Input, Intent, Method, Step};
use netmodel::MethodState;

use crate::{Config, Mode, Static, View};

impl<A: Copy + Into<IpAddr>> Static<A> {
    /// Creates a static method from the given configuration.
    #[must_use = "the method owns the provisioning state"]
    pub const fn new(config: Config<A>) -> Self {
        Self {
            config,
            started: false,
            pending: None,
            cursor: 0,
            notice: None,
        }
    }

    /// Returns the configuration the method was created with.
    #[must_use = "callers need the configuration to report it"]
    pub const fn config(&self) -> &Config<A> {
        &self.config
    }

    const fn apply(&mut self, command: Command) {
        match command {
            Command::Start | Command::Reconcile | Command::LinkUp => {
                self.started = true;
                self.begin(Mode::Provision);
            }
            Command::LinkDown => self.notice = Some(MethodState::Degraded),
            Command::Stop => {
                self.started = false;
                self.begin(Mode::Stop);
            }
        }
    }

    const fn begin(&mut self, mode: Mode) {
        self.pending = Some(mode);
        self.cursor = 0;
        self.notice = None;
    }

    fn next_event(&mut self) -> Option<Event> {
        if let Some(event) = self.batch_event() {
            return Some(event);
        }

        self.notice.take().map(Event::State)
    }

    fn batch_event(&mut self) -> Option<Event> {
        let mode = self.pending?;
        let event = Self::event_of(&self.config, mode, self.cursor)?;
        self.cursor = self.cursor.saturating_add(1);
        if Self::event_of(&self.config, mode, self.cursor).is_none() {
            self.pending = None;
        }

        Some(event)
    }

    fn event_of(config: &Config<A>, mode: Mode, index: usize) -> Option<Event> {
        let stop = mode == Mode::Stop;
        let addresses = config.addresses();
        if let Some(&(address, prefix_len)) = addresses.get(index) {
            let intent = Self::intent(address, prefix_len, stop);

            return Some(Event::Intent(intent));
        }

        let tail = index.checked_sub(addresses.len())?;
        let event = match (config.gateway(), tail) {
            (Some(gateway), 0) => Event::Intent(Self::route(gateway.into(), stop)),
            (Some(_), 1) | (None, 0) => Event::State(Self::terminal(stop)),
            _ => return None,
        };

        Some(event)
    }

    fn intent(address: A, prefix_len: u8, stop: bool) -> Intent {
        let config = AddressConfig {
            address: address.into(),
            prefix_len,
        };
        if stop {
            Intent::Deconfigure(config)
        } else {
            Intent::Configure(config)
        }
    }

    const fn route(gateway: IpAddr, stop: bool) -> Intent {
        if stop {
            Intent::RemoveDefaultRoute(gateway)
        } else {
            Intent::EnsureDefaultRoute(gateway)
        }
    }

    const fn terminal(stop: bool) -> MethodState {
        if stop {
            MethodState::Stopped
        } else {
            MethodState::Bound
        }
    }
}

impl<A: Copy + Into<IpAddr>> Method for Static<A> {
    type View = View<A>;

    fn step(&mut self, _now: Instant, _wall: SystemTime, input: Input<'_>) -> Step<'_> {
        if let Input::Command(command) = input {
            self.apply(command);
        }

        match self.next_event() {
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
        let &(address, prefix_len) = self.config.addresses().first()?;

        View::new(
            address,
            prefix_len,
            self.config.gateway(),
            self.config.dns(),
        )
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
    const ADDRESSES: &[(Ipv4Addr, u8)] = &[(ADDRESS_A, 24), (ADDRESS_B, 24)];
    const DNS_SERVERS: &[Ipv4Addr] = &[DNS];

    fn config() -> Config<Ipv4Addr> {
        Config::new(ADDRESSES, Some(GATEWAY), DNS_SERVERS).expect("within capacity")
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
            method.config().addresses(),
            &[(ADDRESS_A, 24), (ADDRESS_B, 24)]
        );
        assert_eq!(method.config().gateway(), Some(GATEWAY));
        assert_eq!(method.config().dns(), &[DNS]);
    }

    #[test]
    fn start_emits_configure_route_and_bound() {
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
    fn stop_emits_deconfigure_route_and_stopped() {
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
    fn reconcile_emits_configure_route_and_bound() {
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
        let mut method = Static::<Ipv4Addr>::new(
            Config::new(&[(ADDRESS_A, 24), (ADDRESS_B, 24)], None, &[]).expect("within capacity"),
        );
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
        let mut method =
            Static::<Ipv4Addr>::new(Config::new(&[], None, &[]).expect("within capacity"));

        // ACT
        let event = command(&mut method, Command::Start);
        let view = method.view();

        // ASSERT
        assert!(matches!(event, Event::State(MethodState::Bound)));
        assert!(view.is_none());
    }

    #[test]
    fn link_up_emits_configure() {
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

    #[test]
    fn new_command_replaces_undrained_batch() {
        // ARRANGE
        let mut method = Static::<Ipv4Addr>::new(config());
        drop(command(&mut method, Command::Start));

        // ACT
        let first = intent_of(&command(&mut method, Command::Stop));
        let second = expect_intent(&mut method);
        let route = expect_intent(&mut method);
        let state = next_event(&mut method);
        let idle = send(&mut method, Input::Poll);

        // ASSERT
        assert_deconfigure(first, ADDRESS_A);
        assert_deconfigure(second, ADDRESS_B);
        assert!(matches!(
            route,
            Intent::RemoveDefaultRoute(IpAddr::V4(GATEWAY))
        ));
        assert!(matches!(state, Event::State(MethodState::Stopped)));
        assert!(idle.is_none());
    }

    #[test]
    fn link_down_keeps_undrained_stop_batch() {
        // ARRANGE
        let mut method = Static::<Ipv4Addr>::new(config());
        drop(command(&mut method, Command::Start));
        drain_events(&mut method, 3);
        drop(command(&mut method, Command::Stop));

        // ACT
        let second = intent_of(&command(&mut method, Command::LinkDown));
        let route = expect_intent(&mut method);
        let stopped = next_event(&mut method);
        let degraded = next_event(&mut method);
        let idle = send(&mut method, Input::Poll);

        // ASSERT
        assert_deconfigure(second, ADDRESS_B);
        assert!(matches!(
            route,
            Intent::RemoveDefaultRoute(IpAddr::V4(GATEWAY))
        ));
        assert!(matches!(stopped, Event::State(MethodState::Stopped)));
        assert!(matches!(degraded, Event::State(MethodState::Degraded)));
        assert!(idle.is_none());
    }

    #[test]
    fn newer_command_clears_stale_notice() {
        // ARRANGE
        let mut method = Static::<Ipv4Addr>::new(config());
        drop(command(&mut method, Command::Start));
        drop(command(&mut method, Command::LinkDown));

        // ACT
        let first = intent_of(&command(&mut method, Command::Stop));
        let second = expect_intent(&mut method);
        let route = expect_intent(&mut method);
        let state = next_event(&mut method);
        let idle = send(&mut method, Input::Poll);

        // ASSERT
        assert_deconfigure(first, ADDRESS_A);
        assert_deconfigure(second, ADDRESS_B);
        assert!(matches!(
            route,
            Intent::RemoveDefaultRoute(IpAddr::V4(GATEWAY))
        ));
        assert!(matches!(state, Event::State(MethodState::Stopped)));
        assert!(idle.is_none());
    }

    #[test]
    fn view_copies_config_dns() {
        // ARRANGE
        let mut method = Static::<Ipv4Addr>::new(config());
        drop(command(&mut method, Command::Start));

        // ACT
        let first = method.view().expect("view");
        let second = method.view().expect("view");

        // ASSERT
        assert_eq!(first.dns(), DNS_SERVERS);
        assert_eq!(second.dns(), DNS_SERVERS);
        assert_eq!(first, second);
    }

    fn assert_view(view: Option<View<Ipv4Addr>>) {
        let view = view.expect("view");
        assert_eq!(view.address(), ADDRESS_A);
        assert_eq!(view.prefix_len(), 24);
        assert_eq!(view.gateway(), Some(GATEWAY));
        assert_eq!(view.dns(), &[DNS]);
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
