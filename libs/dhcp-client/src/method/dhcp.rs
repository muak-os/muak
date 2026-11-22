//! `DHCPv4` address method state machine.

use std::time::Instant;

use addrconf::{Command, Event, Method, Step};

use super::timer::Deadlines;
use super::{Dhcp, Operations, Phase, View, operations};

impl<E: Operations> Method for Dhcp<E> {
    type View = View;

    async fn next_event(&mut self, now: Instant) -> Event {
        run(self, now).await
    }

    fn next_wake(&self, now: Instant) -> Option<Instant> {
        match self.phase {
            Phase::Acquiring => self.retry_at.filter(|at| *at > now),
            Phase::Bound => self
                .deadlines
                .next()
                .or_else(|| {
                    let lease = self.lease.as_ref()?;
                    Deadlines::preview(now, lease)
                })
                .filter(|at| *at > now),
            Phase::Renewing | Phase::Rebinding | Phase::Idle | Phase::Stopped | Phase::Degraded => {
                None
            }
        }
    }

    fn on_command(&mut self, command: Command) {
        match command {
            Command::Start => self.start(),
            Command::Stop => self.stop(),
            Command::Reconcile => self.reconcile(),
            Command::LinkUp => self.link_up(),
            Command::LinkDown => self.link_down(),
        }
    }

    fn view(&self) -> Option<Self::View> {
        let lease = self.lease.clone()?;
        Some(View {
            lease,
            phase: self.phase,
        })
    }
}

async fn run<E: Operations>(method: &mut Dhcp<E>, now: Instant) -> Event {
    loop {
        if let Some(event) = method.queue.pop_front() {
            return event;
        }
        if step(method, now).await == Step::Wait {
            return core::future::pending().await;
        }
    }
}

async fn step<E: Operations>(method: &mut Dhcp<E>, now: Instant) -> Step {
    match method.phase {
        Phase::Acquiring => operations::acquire(method, now).await,
        Phase::Bound => method.wait_bound(now),
        Phase::Renewing => operations::renew(method, now).await,
        Phase::Rebinding => operations::rebind(method, now).await,
        Phase::Idle | Phase::Stopped | Phase::Degraded => Step::Wait,
    }
}

#[cfg(test)]
mod tests {
    use alloc::collections::VecDeque;
    use core::future::Future;
    use core::net::{IpAddr, Ipv4Addr};
    use core::task::{Context, Waker};
    use core::time::Duration;
    use std::sync::Mutex;
    use std::time::{Instant, SystemTime};

    use addrconf::{Command, Event, Intent, Method as _, State};

    use super::{Dhcp, Operations, Phase, View};
    use crate::Failure as DhcpFailure;
    use crate::lease::Lease;

    const MAC: [u8; 6] = [0xAA; 6];
    const SERVER: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);
    const GATEWAY: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);
    const ADDRESS_A: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 2);
    const ADDRESS_B: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 3);

    type Results = Mutex<VecDeque<crate::Result<Lease>>>;

    #[derive(Default)]
    struct Mock {
        acquire: Results,
        renew: Results,
        rebind: Results,
    }

    struct Stalled;

    impl Operations for Mock {
        fn acquire(
            &self,
            _interface: &str,
            _mac: [u8; 6],
        ) -> impl Future<Output = crate::Result<Lease>> + Send {
            core::future::ready(
                self.acquire
                    .lock()
                    .expect("acquire queue")
                    .pop_front()
                    .expect("canned acquire"),
            )
        }

        fn renew(
            &self,
            _interface: &str,
            _mac: [u8; 6],
            _server: Ipv4Addr,
            _assigned: Ipv4Addr,
        ) -> impl Future<Output = crate::Result<Lease>> + Send {
            core::future::ready(
                self.renew
                    .lock()
                    .expect("renew queue")
                    .pop_front()
                    .expect("canned renew"),
            )
        }

        fn rebind(
            &self,
            _interface: &str,
            _mac: [u8; 6],
            _server: Ipv4Addr,
            _assigned: Ipv4Addr,
        ) -> impl Future<Output = crate::Result<Lease>> + Send {
            core::future::ready(
                self.rebind
                    .lock()
                    .expect("rebind queue")
                    .pop_front()
                    .expect("canned rebind"),
            )
        }
    }

    impl Operations for Stalled {
        fn acquire(
            &self,
            _interface: &str,
            _mac: [u8; 6],
        ) -> impl Future<Output = crate::Result<Lease>> + Send {
            core::future::pending()
        }

        fn renew(
            &self,
            _interface: &str,
            _mac: [u8; 6],
            _server: Ipv4Addr,
            _assigned: Ipv4Addr,
        ) -> impl Future<Output = crate::Result<Lease>> + Send {
            core::future::pending()
        }

        fn rebind(
            &self,
            _interface: &str,
            _mac: [u8; 6],
            _server: Ipv4Addr,
            _assigned: Ipv4Addr,
        ) -> impl Future<Output = crate::Result<Lease>> + Send {
            core::future::pending()
        }
    }

    fn mock(
        acquire: Vec<crate::Result<Lease>>,
        renew: Vec<crate::Result<Lease>>,
        rebind: Vec<crate::Result<Lease>>,
    ) -> Mock {
        Mock {
            acquire: Mutex::new(acquire.into()),
            renew: Mutex::new(renew.into()),
            rebind: Mutex::new(rebind.into()),
        }
    }

    fn lease(address: Ipv4Addr) -> Lease {
        Lease {
            obtained_at: SystemTime::UNIX_EPOCH,
            lease_time: Duration::from_secs(100),
            renewal_time: Duration::from_secs(50),
            rebind_time: Duration::from_secs(87),
            server_ip: SERVER,
            assigned_ip: address,
            prefix_len: 24,
            gateway: Some(GATEWAY),
            dns_servers: Vec::new(),
        }
    }

    fn at(now: Instant, secs: u64) -> Instant {
        now.checked_add(Duration::from_secs(secs))
            .expect("representable instant")
    }

    fn is_pending<F: Future>(future: F) -> bool {
        let mut future = core::pin::pin!(future);
        let mut context = Context::from_waker(Waker::noop());

        future.as_mut().poll(&mut context).is_pending()
    }

    async fn next_until_state(method: &mut Dhcp<Mock>, wanted: State, now: Instant) -> Vec<Event> {
        let mut events = Vec::new();
        let mut event = method.next_event(now).await;
        while !matches!(&event, Event::State(state) if *state == wanted) {
            events.push(event);
            event = method.next_event(now).await;
        }
        events.push(event);

        events
    }

    fn arm(method: &mut Dhcp<Mock>, now: Instant) {
        assert!(is_pending(method.next_event(now)));
    }

    async fn bound(now: Instant) -> Dhcp<Mock> {
        let mut method = Dhcp::new(
            mock(vec![Ok(lease(ADDRESS_A))], Vec::new(), Vec::new()),
            "eth0",
            MAC,
        );
        method.on_command(Command::Start);
        next_until_state(&mut method, State::Bound, now).await;
        arm(&mut method, now);

        method
    }

    fn has_configure(events: &[Event], address: Ipv4Addr) -> bool {
        events.iter().any(|event| {
            matches!(
                event,
                Event::Intent(Intent::Configure(config)) if config.address == IpAddr::V4(address)
            )
        })
    }

    fn has_route(events: &[Event]) -> bool {
        events.iter().any(|event| {
            matches!(
                event,
                Event::Intent(Intent::EnsureDefaultRoute(IpAddr::V4(GATEWAY)))
            )
        })
    }

    #[tokio::test]
    async fn start_configures_route_binds_and_views() {
        // ARRANGE
        let now = Instant::now();
        let mut method = Dhcp::new(
            mock(vec![Ok(lease(ADDRESS_A))], Vec::new(), Vec::new()),
            "eth0",
            MAC,
        );

        // ACT
        method.on_command(Command::Start);
        let events = next_until_state(&mut method, State::Bound, now).await;
        let view = method.view();

        // ASSERT
        assert!(has_configure(&events, ADDRESS_A));
        assert!(has_route(&events));
        assert!(matches!(
            view,
            Some(View {
                lease,
                phase: Phase::Bound
            }) if lease.assigned_ip == ADDRESS_A
        ));
    }

    #[tokio::test]
    async fn renew_timer_replaces_the_lease() {
        // ARRANGE
        let now = Instant::now();
        let mut method = Dhcp::new(
            mock(
                vec![Ok(lease(ADDRESS_A))],
                vec![Ok(lease(ADDRESS_B))],
                Vec::new(),
            ),
            "eth0",
            MAC,
        );
        method.on_command(Command::Start);
        next_until_state(&mut method, State::Bound, now).await;
        arm(&mut method, now);

        // ACT
        let renew_at = at(now, 50);
        let events = next_until_state(&mut method, State::Bound, renew_at).await;
        let wake = method.next_wake(renew_at);
        let view = method.view();

        // ASSERT
        assert!(has_configure(&events, ADDRESS_B));
        assert_eq!(wake, Some(at(renew_at, 50)));
        assert!(matches!(
            view,
            Some(View {
                lease,
                phase: Phase::Bound
            }) if lease.assigned_ip == ADDRESS_B
        ));
    }

    #[tokio::test]
    async fn expire_deconfigures_and_reacquires() {
        // ARRANGE
        let now = Instant::now();
        let mut method = bound(now).await;
        let expiry = at(now, 100);

        // ACT
        let first = method.next_event(expiry).await;
        let second = method.next_event(expiry).await;
        let third = method.next_event(expiry).await;
        let view = method.view();

        // ASSERT
        assert!(matches!(
            first,
            Event::Intent(Intent::Deconfigure(config))
                if config.address == IpAddr::V4(ADDRESS_A)
        ));
        assert!(matches!(
            second,
            Event::Intent(Intent::RemoveDefaultRoute(IpAddr::V4(GATEWAY)))
        ));
        assert!(matches!(third, Event::State(State::Acquiring)));
        assert!(view.is_none());
    }

    #[tokio::test]
    async fn renewal_failure_rebinds() {
        // ARRANGE
        let now = Instant::now();
        let mut method = Dhcp::new(
            mock(
                vec![Ok(lease(ADDRESS_A))],
                vec![Err(DhcpFailure::Timeout)],
                vec![Ok(lease(ADDRESS_B))],
            ),
            "eth0",
            MAC,
        );
        method.on_command(Command::Start);
        next_until_state(&mut method, State::Bound, now).await;
        arm(&mut method, now);
        let renew_at = at(now, 50);

        // ACT
        let warning = method.next_event(renew_at).await;
        let after_failed_renew = method.view();
        let wake = method.next_wake(renew_at);
        let events = next_until_state(&mut method, State::Bound, at(now, 87)).await;
        let view = method.view();

        // ASSERT
        assert!(matches!(warning, Event::Warning(_)));
        assert!(matches!(
            after_failed_renew,
            Some(View {
                lease,
                phase: Phase::Bound
            }) if lease.assigned_ip == ADDRESS_A
        ));
        assert_eq!(wake, Some(at(now, 87)));
        assert!(has_configure(&events, ADDRESS_B));
        assert!(matches!(
            view,
            Some(View {
                lease,
                phase: Phase::Bound
            }) if lease.assigned_ip == ADDRESS_B
        ));
    }

    #[tokio::test]
    async fn rebind_failure_expires() {
        // ARRANGE
        let now = Instant::now();
        let mut method = Dhcp::new(
            mock(
                vec![Ok(lease(ADDRESS_A))],
                Vec::new(),
                vec![Err(DhcpFailure::Timeout)],
            ),
            "eth0",
            MAC,
        );
        method.on_command(Command::Start);
        next_until_state(&mut method, State::Bound, now).await;
        arm(&mut method, now);
        let rebind_at = at(now, 87);

        // ACT
        let warning = method.next_event(rebind_at).await;
        let wake = method.next_wake(rebind_at);
        let expiry = at(now, 100);
        let first = method.next_event(expiry).await;
        let second = method.next_event(expiry).await;
        let third = method.next_event(expiry).await;
        let view = method.view();

        // ASSERT
        assert!(matches!(warning, Event::Warning(_)));
        assert_eq!(wake, Some(at(now, 100)));
        assert!(matches!(
            first,
            Event::Intent(Intent::Deconfigure(config))
                if config.address == IpAddr::V4(ADDRESS_A)
        ));
        assert!(matches!(
            second,
            Event::Intent(Intent::RemoveDefaultRoute(IpAddr::V4(GATEWAY)))
        ));
        assert!(matches!(third, Event::State(State::Acquiring)));
        assert!(view.is_none());
    }

    #[tokio::test]
    async fn link_up_after_link_down_reapplies_the_lease() {
        // ARRANGE
        let now = Instant::now();
        let mut method = bound(now).await;
        method.on_command(Command::LinkDown);
        method.next_event(now).await;

        // ACT
        method.on_command(Command::LinkUp);
        let first = method.next_event(now).await;
        let second = method.next_event(now).await;
        let third = method.next_event(now).await;

        // ASSERT
        assert!(matches!(
            first,
            Event::Intent(Intent::Configure(config))
                if config.address == IpAddr::V4(ADDRESS_A)
        ));
        assert!(matches!(
            second,
            Event::Intent(Intent::EnsureDefaultRoute(IpAddr::V4(GATEWAY)))
        ));
        assert!(matches!(third, Event::State(State::Bound)));
    }

    #[tokio::test]
    async fn reconcile_without_lease_restarts_discovery() {
        // ARRANGE
        let now = Instant::now();
        let mut method = Dhcp::new(
            mock(vec![Ok(lease(ADDRESS_A))], Vec::new(), Vec::new()),
            "eth0",
            MAC,
        );

        // ACT
        method.on_command(Command::Reconcile);
        let event = method.next_event(now).await;
        let view = method.view();

        // ASSERT
        assert!(matches!(event, Event::State(State::Acquiring)));
        assert!(view.is_none());
    }

    #[tokio::test]
    async fn nak_restarts_discovery() {
        // ARRANGE
        let now = Instant::now();
        let mut method = Dhcp::new(
            mock(
                vec![Ok(lease(ADDRESS_A)), Ok(lease(ADDRESS_B))],
                vec![Err(DhcpFailure::Nak)],
                Vec::new(),
            ),
            "eth0",
            MAC,
        );
        method.on_command(Command::Start);
        next_until_state(&mut method, State::Bound, now).await;
        arm(&mut method, now);

        // ACT
        let renew_at = at(now, 50);
        let restarted = method.next_event(renew_at).await;
        let reconfigured = method.next_event(renew_at).await;

        // ASSERT
        assert!(matches!(restarted, Event::State(State::Acquiring)));
        assert!(matches!(
            reconfigured,
            Event::Intent(Intent::Configure(config))
                if config.address == IpAddr::V4(ADDRESS_B)
        ));
    }

    #[tokio::test]
    async fn link_down_degrades_and_keeps_the_lease() {
        // ARRANGE
        let now = Instant::now();
        let mut method = bound(now).await;

        // ACT
        method.on_command(Command::LinkDown);
        let event = method.next_event(now).await;
        let view = method.view();

        // ASSERT
        assert!(matches!(event, Event::State(State::Degraded)));
        assert!(matches!(view, Some(View { .. })));
    }

    #[tokio::test]
    async fn stop_deconfigures_and_stops() {
        // ARRANGE
        let now = Instant::now();
        let mut method = bound(now).await;

        // ACT
        method.on_command(Command::Stop);
        let first = method.next_event(now).await;
        let second = method.next_event(now).await;
        let third = method.next_event(now).await;
        let view = method.view();

        // ASSERT
        assert!(matches!(first, Event::Intent(Intent::Deconfigure(_))));
        assert!(matches!(
            second,
            Event::Intent(Intent::RemoveDefaultRoute(_))
        ));
        assert!(matches!(third, Event::State(State::Stopped)));
        assert!(view.is_none());
    }

    #[tokio::test]
    async fn bound_waits_for_the_renewal_deadline() {
        // ARRANGE
        let now = Instant::now();
        let mut method = bound(now).await;

        // ACT
        let wake = method.next_wake(now);
        let waiting = is_pending(method.next_event(now));

        // ASSERT
        assert_eq!(wake, Some(at(now, 50)));
        assert!(waiting);
    }

    #[tokio::test]
    async fn bind_defers_arming_to_the_next_bound_step() {
        // ARRANGE
        let now = Instant::now();
        let mut method = Dhcp::new(
            mock(vec![Ok(lease(ADDRESS_A))], Vec::new(), Vec::new()),
            "eth0",
            MAC,
        );
        method.on_command(Command::Start);

        // ACT
        next_until_state(&mut method, State::Bound, now).await;
        let armed = method.deadlines.next();
        let preview = method.next_wake(now);
        let waiting = is_pending(method.next_event(now));
        let armed_wake = method.next_wake(now);

        // ASSERT
        assert_eq!(armed, None);
        assert_eq!(preview, Some(at(now, 50)));
        assert!(waiting);
        assert_eq!(armed_wake, preview);
    }

    #[tokio::test]
    async fn deadlines_anchor_to_the_bound_step_clock() {
        // ARRANGE
        let now = Instant::now();
        let mut method = Dhcp::new(
            mock(vec![Ok(lease(ADDRESS_A))], Vec::new(), Vec::new()),
            "eth0",
            MAC,
        );
        method.on_command(Command::Start);
        next_until_state(&mut method, State::Bound, now).await;

        // ACT
        let bound_at = at(now, 7);
        let waiting = is_pending(method.next_event(bound_at));
        let wake = method.next_wake(bound_at);

        // ASSERT
        assert!(waiting);
        assert_eq!(wake, Some(at(now, 57)));
    }

    #[tokio::test]
    async fn failed_acquire_arms_a_retry_deadline() {
        // ARRANGE
        let now = Instant::now();
        let mut method = Dhcp::new(
            mock(
                vec![Err(DhcpFailure::Timeout), Ok(lease(ADDRESS_A))],
                Vec::new(),
                Vec::new(),
            ),
            "eth0",
            MAC,
        );
        method.on_command(Command::Start);
        let acquiring = method.next_event(now).await;

        // ACT
        let warning = method.next_event(now).await;
        let wake = method.next_wake(now);
        let waiting = is_pending(method.next_event(now));

        // ASSERT
        assert!(matches!(acquiring, Event::State(State::Acquiring)));
        assert!(matches!(warning, Event::Warning(_)));
        assert_eq!(wake, Some(at(now, 2)));
        assert!(waiting);
    }

    #[tokio::test]
    async fn exhausted_acquire_attempts_degrade() {
        // ARRANGE
        let now = Instant::now();
        let mut method = Dhcp::new(
            mock(
                vec![
                    Err(DhcpFailure::Timeout),
                    Err(DhcpFailure::Timeout),
                    Err(DhcpFailure::Timeout),
                ],
                Vec::new(),
                Vec::new(),
            ),
            "eth0",
            MAC,
        );
        method.on_command(Command::Start);
        let _acquiring = method.next_event(now).await;

        // ACT
        let _first = method.next_event(now).await;
        let _second = method.next_event(at(now, 2)).await;
        let failed = method.next_event(at(now, 6)).await;
        let wake = method.next_wake(at(now, 6));

        // ASSERT
        assert!(matches!(failed, Event::Failed(_)));
        assert_eq!(wake, None);
    }

    #[tokio::test]
    async fn reapplied_lease_rearms_its_deadlines() {
        // ARRANGE
        let now = Instant::now();
        let mut method = bound(now).await;
        method.on_command(Command::LinkDown);
        method.next_event(now).await;
        method.on_command(Command::LinkUp);
        let _configure = method.next_event(now).await;
        let _route = method.next_event(now).await;
        let _bound = method.next_event(now).await;

        // ACT
        let predicted = method.next_wake(now);
        let waiting = is_pending(method.next_event(now));
        let wake = method.next_wake(now);

        // ASSERT
        assert_eq!(predicted, Some(at(now, 50)));
        assert!(waiting);
        assert_eq!(wake, Some(at(now, 50)));
    }

    #[tokio::test]
    async fn renewal_after_expiry_deconfigures_instead_of_renewing() {
        // ARRANGE
        let now = Instant::now();
        let mut method = bound(now).await;
        method.phase = Phase::Renewing;

        // ACT
        let first = method.next_event(at(now, 100)).await;

        // ASSERT
        assert!(matches!(
            first,
            Event::Intent(Intent::Deconfigure(config))
                if config.address == IpAddr::V4(ADDRESS_A)
        ));
    }

    #[tokio::test]
    async fn due_retry_starts_the_exchange_without_a_stale_wake() {
        // ARRANGE
        let now = Instant::now();
        let mut method = Dhcp::new(Stalled, "eth0", MAC);
        method.on_command(Command::Start);
        let _started = method.next_event(now).await;
        method.retry_at = Some(now);

        // ACT
        let acquiring = is_pending(method.next_event(now));
        let wake = method.next_wake(now);

        // ASSERT
        assert!(acquiring);
        assert_eq!(wake, None);
    }

    #[tokio::test]
    async fn renewing_and_rebinding_have_no_wake_deadline() {
        // ARRANGE
        let now = Instant::now();
        let mut renewing = Dhcp::new(Stalled, "eth0", MAC);
        renewing.lease = Some(lease(ADDRESS_A));
        renewing.phase = Phase::Renewing;
        let mut rebinding = Dhcp::new(Stalled, "eth0", MAC);
        rebinding.lease = Some(lease(ADDRESS_A));
        rebinding.phase = Phase::Rebinding;

        // ACT
        let renewing_pending = is_pending(renewing.next_event(now));
        let renewing_wake = renewing.next_wake(now);
        let rebinding_pending = is_pending(rebinding.next_event(now));
        let rebinding_wake = rebinding.next_wake(now);

        // ASSERT
        assert!(renewing_pending);
        assert_eq!(renewing_wake, None);
        assert!(rebinding_pending);
        assert_eq!(rebinding_wake, None);
    }

    #[tokio::test]
    async fn idle_and_stopped_have_no_wake_deadline() {
        // ARRANGE
        let now = Instant::now();
        let mut method = Dhcp::new(mock(Vec::new(), Vec::new(), Vec::new()), "eth0", MAC);
        let idle = method.next_wake(now);
        let idle_waiting = is_pending(method.next_event(now));
        method.on_command(Command::Stop);

        // ACT
        let _stopped = method.next_event(now).await;
        let stopped = method.next_wake(now);

        // ASSERT
        assert_eq!(idle, None);
        assert!(idle_waiting);
        assert_eq!(stopped, None);
    }
}
