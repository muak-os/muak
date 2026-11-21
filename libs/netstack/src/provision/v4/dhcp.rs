//! `DHCPv4` address method state machine.

use super::{Dhcp, Exchange};
use crate::provision::{Command, Event, Method};
use crate::view::{DhcpState, V4State};

// Lifecycle phase of the DHCP method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
    Idle,
    Acquiring,
    Bound,
    Renewing,
    Rebinding,
    Degraded,
    Stopped,
}

impl<E: Exchange> Method for Dhcp<E> {
    type View = V4State;

    async fn next_event(&mut self) -> Event {
        while self.queue.is_empty() {
            match self.phase {
                Phase::Acquiring => self.run_acquire().await,
                Phase::Bound => self.wait_bound().await,
                Phase::Renewing => self.run_renew().await,
                Phase::Rebinding => self.run_rebind().await,
                Phase::Idle | Phase::Stopped | Phase::Degraded => self.idle().await,
            }
        }
        match self.queue.pop_front() {
            Some(event) => event,
            None => core::future::pending::<Event>().await,
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
        Some(V4State::Dhcp {
            lease,
            phase: view_phase(self.phase),
        })
    }
}

const fn view_phase(phase: Phase) -> DhcpState {
    match phase {
        Phase::Acquiring => DhcpState::Init,
        Phase::Renewing => DhcpState::Renewing,
        Phase::Rebinding => DhcpState::Rebinding,
        Phase::Idle | Phase::Bound | Phase::Degraded | Phase::Stopped => DhcpState::Bound,
    }
}

#[cfg(test)]
mod tests {
    use alloc::collections::VecDeque;
    use core::future::Future;
    use core::net::{IpAddr, Ipv4Addr};
    use core::time::Duration;
    use std::sync::Mutex;
    use std::time::SystemTime;

    use dhcp_client::Failure as DhcpFailure;
    use dhcp_client::lease::Lease;
    use tokio::time::{advance, timeout};

    use super::{Dhcp, Exchange};
    use crate::provision::{Command, Event, Intent, Method as _, State};
    use crate::view::{DhcpState, V4State};

    const MAC: [u8; 6] = [0xAA; 6];
    const SERVER: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);
    const GATEWAY: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);
    const ADDRESS_A: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 2);
    const ADDRESS_B: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 3);

    type Results = Mutex<VecDeque<dhcp_client::Result<Lease>>>;

    #[derive(Default)]
    struct Mock {
        acquire: Results,
        renew: Results,
        rebind: Results,
    }

    impl Exchange for Mock {
        fn acquire(
            &self,
            _interface: &str,
            _mac: [u8; 6],
        ) -> impl Future<Output = dhcp_client::Result<Lease>> + Send {
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
        ) -> impl Future<Output = dhcp_client::Result<Lease>> + Send {
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
        ) -> impl Future<Output = dhcp_client::Result<Lease>> + Send {
            core::future::ready(
                self.rebind
                    .lock()
                    .expect("rebind queue")
                    .pop_front()
                    .expect("canned rebind"),
            )
        }
    }

    fn mock(
        acquire: Vec<dhcp_client::Result<Lease>>,
        renew: Vec<dhcp_client::Result<Lease>>,
        rebind: Vec<dhcp_client::Result<Lease>>,
    ) -> Mock {
        Mock {
            acquire: Mutex::new(acquire.into()),
            renew: Mutex::new(renew.into()),
            rebind: Mutex::new(rebind.into()),
        }
    }

    async fn quiet_for(method: &mut Dhcp<Mock>, window: Duration) -> Option<Event> {
        timeout(window, method.next_event()).await.ok()
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

    async fn next_until_state(method: &mut Dhcp<Mock>, wanted: State) -> Vec<Event> {
        let mut events = Vec::new();
        events.push(method.next_event().await);
        while !matches!(events.last(), Some(Event::State(state)) if *state == wanted) {
            events.push(method.next_event().await);
        }
        events
    }

    async fn bound() -> Dhcp<Mock> {
        let mut method = Dhcp::new(
            mock(vec![Ok(lease(ADDRESS_A))], Vec::new(), Vec::new()),
            "eth0",
            MAC,
        );
        method.on_command(Command::Start);
        next_until_state(&mut method, State::Bound).await;

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

    #[tokio::test(start_paused = true)]
    async fn start_configures_route_binds_and_views() {
        // ARRANGE
        let mut method = Dhcp::new(
            mock(vec![Ok(lease(ADDRESS_A))], Vec::new(), Vec::new()),
            "eth0",
            MAC,
        );

        // ACT
        method.on_command(Command::Start);
        let events = next_until_state(&mut method, State::Bound).await;
        let view = method.view();

        // ASSERT
        assert!(has_configure(&events, ADDRESS_A));
        assert!(has_route(&events));
        assert!(matches!(
            view,
            Some(V4State::Dhcp {
                lease,
                phase: DhcpState::Bound
            }) if lease.assigned_ip == ADDRESS_A
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn renew_timer_replaces_the_lease() {
        // ARRANGE
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
        next_until_state(&mut method, State::Bound).await;

        // ACT
        advance(Duration::from_secs(50)).await;
        let events = next_until_state(&mut method, State::Bound).await;
        let view = method.view();

        // ASSERT
        assert!(has_configure(&events, ADDRESS_B));
        assert!(matches!(
            view,
            Some(V4State::Dhcp {
                lease,
                phase: DhcpState::Bound
            }) if lease.assigned_ip == ADDRESS_B
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn expire_deconfigures_and_reacquires() {
        // ARRANGE
        let mut method = bound().await;

        // ACT
        advance(Duration::from_secs(100)).await;
        let first = method.next_event().await;
        let second = method.next_event().await;
        let third = method.next_event().await;
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

    #[tokio::test(start_paused = true)]
    async fn renewal_failure_rebinds() {
        // ARRANGE
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
        next_until_state(&mut method, State::Bound).await;

        // ACT
        advance(Duration::from_secs(50)).await;
        let quiet = quiet_for(&mut method, Duration::from_secs(10)).await;
        let after_failed_renew = method.view();
        advance(Duration::from_secs(27)).await;
        let events = next_until_state(&mut method, State::Bound).await;
        let view = method.view();

        // ASSERT
        assert!(quiet.is_none(), "a failed renew emits no event");
        assert!(matches!(
            after_failed_renew,
            Some(V4State::Dhcp {
                lease,
                phase: DhcpState::Bound
            }) if lease.assigned_ip == ADDRESS_A
        ));
        assert!(has_configure(&events, ADDRESS_B));
        assert!(matches!(
            view,
            Some(V4State::Dhcp {
                lease,
                phase: DhcpState::Bound
            }) if lease.assigned_ip == ADDRESS_B
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn rebind_failure_expires() {
        // ARRANGE
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
        next_until_state(&mut method, State::Bound).await;

        // ACT
        advance(Duration::from_secs(87)).await;
        let quiet = quiet_for(&mut method, Duration::from_secs(5)).await;
        advance(Duration::from_secs(13)).await;
        let first = method.next_event().await;
        let second = method.next_event().await;
        let third = method.next_event().await;
        let view = method.view();

        // ASSERT
        assert!(quiet.is_none(), "a failed rebind emits no event");
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

    #[tokio::test(start_paused = true)]
    async fn link_up_after_link_down_reapplies_the_lease() {
        // ARRANGE
        let mut method = bound().await;
        method.on_command(Command::LinkDown);
        method.next_event().await;

        // ACT
        method.on_command(Command::LinkUp);
        let first = method.next_event().await;
        let second = method.next_event().await;
        let third = method.next_event().await;

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

    #[tokio::test(start_paused = true)]
    async fn reconcile_without_lease_restarts_discovery() {
        // ARRANGE
        let mut method = Dhcp::new(
            mock(vec![Ok(lease(ADDRESS_A))], Vec::new(), Vec::new()),
            "eth0",
            MAC,
        );

        // ACT
        method.on_command(Command::Reconcile);
        let event = method.next_event().await;
        let view = method.view();

        // ASSERT
        assert!(matches!(event, Event::State(State::Acquiring)));
        assert!(view.is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn nak_restarts_discovery() {
        // ARRANGE
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
        next_until_state(&mut method, State::Bound).await;

        // ACT
        advance(Duration::from_secs(50)).await;
        let restarted = method.next_event().await;
        let reconfigured = method.next_event().await;

        // ASSERT
        assert!(matches!(restarted, Event::State(State::Acquiring)));
        assert!(matches!(
            reconfigured,
            Event::Intent(Intent::Configure(config))
                if config.address == IpAddr::V4(ADDRESS_B)
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn link_down_degrades_and_keeps_the_lease() {
        // ARRANGE
        let mut method = bound().await;

        // ACT
        method.on_command(Command::LinkDown);
        let event = method.next_event().await;
        let view = method.view();

        // ASSERT
        assert!(matches!(event, Event::State(State::Degraded)));
        assert!(matches!(view, Some(V4State::Dhcp { .. })));
    }

    #[tokio::test(start_paused = true)]
    async fn stop_deconfigures_and_stops() {
        // ARRANGE
        let mut method = bound().await;

        // ACT
        method.on_command(Command::Stop);
        let first = method.next_event().await;
        let second = method.next_event().await;
        let third = method.next_event().await;
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
}
