//! Stateless address autoconfiguration method.

mod intent;
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
use std::time::Instant;

use self::state::{ManagedAddress, ManagedDns, ManagedRouter};
use super::Config;
use crate::provision::{Command, Event, Intent, Method, State, Step};
use crate::transport::icmpv6::Transport;
use crate::view::V6State;

const MAX_RTR_SOLICITATIONS: u32 = 3;
const RTR_SOLICITATION_INTERVAL: Duration = Duration::from_secs(4);

/// Stateless address autoconfiguration method.
pub struct Slaac<T: Transport> {
    interface: String,
    mac: [u8; 6],
    socket: T,
    running: bool,
    solicited: bool,
    solicitation_attempt: u32,
    solicitation_at: Option<Instant>,
    retry_at: Option<Instant>,
    reconcile: bool,
    address: Option<ManagedAddress>,
    router: Option<ManagedRouter>,
    dns_servers: Vec<ManagedDns>,
    fallback_dns: Vec<Ipv6Addr>,
    queue: VecDeque<Event>,
}

impl<T: Transport> Slaac<T> {
    /// Creates a SLAAC method around an already-open `ICMPv6` transport.
    #[must_use = "the method owns the provisioning state"]
    pub fn new(config: Config, interface: String, mac: [u8; 6], socket: T) -> Self {
        Self {
            interface,
            mac,
            socket,
            running: false,
            solicited: false,
            solicitation_attempt: 0,
            solicitation_at: None,
            retry_at: None,
            reconcile: false,
            address: None,
            router: None,
            dns_servers: Vec::new(),
            fallback_dns: config.dns,
            queue: VecDeque::new(),
        }
    }

    fn dns_fallback(&self, now: Instant) -> Vec<ManagedDns> {
        self.fallback_dns
            .iter()
            .map(|&server| ManagedDns::new(server, u64::from(u32::MAX), now))
            .collect()
    }

    const fn resolicit(&mut self) {
        self.running = true;
        self.solicited = false;
        self.solicitation_attempt = 0;
        self.solicitation_at = None;
        self.retry_at = None;
    }

    const fn suspend(&mut self) {
        self.running = false;
    }

    fn reapply(&mut self, now: Instant) {
        if !self.running {
            return;
        }
        let Some(address) = self
            .address
            .as_ref()
            .filter(|address| address.is_valid(now))
        else {
            return;
        };
        let intent = intent::configure(address.address, address.prefix_len);
        let router = self
            .router
            .as_ref()
            .filter(|router| router.is_valid(now))
            .map(|router| router.value);
        self.queue.push_back(Event::Intent(intent));
        if let Some(router) = router {
            self.queue
                .push_back(Event::Intent(Intent::EnsureDefaultRoute(IpAddr::V6(
                    router,
                ))));
        }
    }

    fn stop(&mut self) {
        self.running = false;
        self.solicited = true;
        self.solicitation_at = None;
        self.retry_at = None;
        if let Some(address) = self.address.take() {
            let intent = intent::deconfigure(address.address, address.prefix_len);
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

    async fn next_event(&mut self, now: Instant) -> Event {
        run(self, now).await
    }

    fn next_wake(&self, now: Instant) -> Option<Instant> {
        timers::next_wake(self, now)
    }

    fn on_command(&mut self, command: Command) {
        match command {
            Command::Start | Command::LinkUp => self.resolicit(),
            Command::Reconcile => self.reconcile = true,
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

async fn run<T: Transport + Send>(method: &mut Slaac<T>, now: Instant) -> Event {
    loop {
        if let Some(event) = method.queue.pop_front() {
            return event;
        }
        if step(method, now).await == Step::Wait {
            return core::future::pending().await;
        }
    }
}

async fn step<T: Transport>(method: &mut Slaac<T>, now: Instant) -> Step {
    if method.reconcile {
        method.reconcile = false;
        method.reapply(now);
        return Step::Advance;
    }
    if !method.running {
        return Step::Wait;
    }
    if !method.solicited {
        return solicit::initial(method, now).await;
    }

    solicit::monitor(method, now).await
}

#[cfg(test)]
mod tests {
    use alloc::collections::VecDeque;
    use alloc::string::String;
    use alloc::vec;
    use alloc::vec::Vec;
    use core::future::Future;
    use core::net::{IpAddr, Ipv6Addr};
    use core::task::{Context, Waker};
    use core::time::Duration;
    use std::sync::Mutex;
    use std::time::Instant;

    use super::{RTR_SOLICITATION_INTERVAL, Slaac};
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

    fn at(now: Instant, secs: u64) -> Instant {
        now.checked_add(Duration::from_secs(secs))
            .expect("representable instant")
    }

    fn is_pending<F: Future>(future: F) -> bool {
        let mut future = core::pin::pin!(future);
        let mut context = Context::from_waker(Waker::noop());

        future.as_mut().poll(&mut context).is_pending()
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

    async fn bound(transport: MockTransport, now: Instant) -> Slaac<MockTransport> {
        transport.push_ra(ra(1800, Some((7200, 3600)), &[]), ROUTER);
        let mut method = new_method(transport);
        method.on_command(Command::Start);
        let _configure = method.next_event(now).await;
        let _route = method.next_event(now).await;
        let _bound = method.next_event(now).await;

        method
    }

    #[tokio::test]
    async fn solicitation_sends_rs_and_configures_address() {
        // ARRANGE
        let now = Instant::now();
        let transport = MockTransport::default();
        transport.push_ra(ra(1800, Some((7200, 3600)), &[]), ROUTER);
        let mut method = new_method(transport);

        // ACT
        method.on_command(Command::Start);
        let configure = method.next_event(now).await;
        let route = method.next_event(now).await;
        let bound = method.next_event(now).await;

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

    #[tokio::test]
    async fn missing_router_advertisement_warns_after_three_attempts() {
        // ARRANGE
        let now = Instant::now();
        let mut method = new_method(MockTransport::default());

        // ACT
        method.on_command(Command::Start);
        let silence = is_pending(method.next_event(now));
        let first_wake = method.next_wake(now);
        let sent_after_first = method.socket.destinations().len();
        let _second = is_pending(method.next_event(at(now, 4)));
        let second_wake = method.next_wake(at(now, 4));
        let _third = is_pending(method.next_event(at(now, 8)));
        let warning = method.next_event(at(now, 12)).await;
        let exhausted_wake = method.next_wake(at(now, 12));

        // ASSERT
        assert!(silence);
        assert_eq!(first_wake, Some(at(now, 4)));
        assert_eq!(sent_after_first, 1);
        assert_eq!(second_wake, Some(at(now, 8)));
        assert_eq!(exhausted_wake, None);
        assert_eq!(method.socket.destinations().len(), 3);
        assert!(matches!(&warning, Event::Warning(text) if text.contains("continuing to listen")));
        assert!(method.view().is_none());
    }

    #[tokio::test]
    async fn resolicit_wakes_after_the_solicitation_interval() {
        // ARRANGE
        let now = Instant::now();
        let mut method = new_method(MockTransport::default());

        // ACT
        method.on_command(Command::Start);
        let wake = method.next_wake(now);
        let silence = is_pending(method.next_event(now));

        // ASSERT
        assert_eq!(wake, Some(at(now, 4)));
        assert!(silence);
        assert_eq!(method.socket.destinations().len(), 1);
    }

    #[tokio::test]
    async fn stopped_method_has_no_wake_deadline() {
        // ARRANGE
        let now = Instant::now();
        let mut method = bound(MockTransport::default(), now).await;

        // ACT
        method.on_command(Command::Stop);
        let wake = method.next_wake(now);

        // ASSERT
        assert_eq!(wake, None);
    }

    #[tokio::test]
    async fn solicitation_schedule_is_one_router_solicitation_per_interval() {
        // ARRANGE
        let now = Instant::now();
        let mut method = new_method(MockTransport::default());

        // ACT
        method.on_command(Command::Start);
        let _silence = is_pending(method.next_event(now));
        let _second = is_pending(method.next_event(at(now, 4)));
        let _third = is_pending(method.next_event(at(now, 8)));

        // ASSERT
        assert_eq!(method.socket.destinations().len(), 3);
        assert_eq!(
            method.next_wake(at(now, 8)),
            Some(at(now, 8) + RTR_SOLICITATION_INTERVAL)
        );
    }

    #[tokio::test]
    async fn reconcile_reemits_configuration_without_soliciting() {
        // ARRANGE
        let now = Instant::now();
        let mut method = bound(MockTransport::default(), now).await;

        // ACT
        method.on_command(Command::Reconcile);
        let configure = method.next_event(now).await;
        let route = method.next_event(now).await;
        let silence = is_pending(method.next_event(now));

        // ASSERT
        assert!(matches!(
            configure,
            Event::Intent(Intent::Configure(config))
                if config.address == IpAddr::V6(ADDRESS) && config.prefix_len == 64
        ));
        assert!(matches!(
            route,
            Event::Intent(Intent::EnsureDefaultRoute(IpAddr::V6(ROUTER)))
        ));
        assert_eq!(
            method.socket.destinations(),
            vec![::slaac::icmpv6::ALL_ROUTERS]
        );
        assert!(silence);
    }

    #[tokio::test]
    async fn reconcile_without_configuration_does_nothing() {
        // ARRANGE
        let now = Instant::now();
        let mut method = new_method(MockTransport::default());
        method.on_command(Command::Start);
        let _silence = is_pending(method.next_event(now));
        let destinations = method.socket.destinations();

        // ACT
        method.on_command(Command::Reconcile);
        let silence = is_pending(method.next_event(now));

        // ASSERT
        assert!(silence);
        assert!(method.view().is_none());
        assert_eq!(method.socket.destinations(), destinations);
    }

    #[tokio::test]
    async fn link_up_after_suspend_resolicits() {
        // ARRANGE
        let now = Instant::now();
        let mut method = bound(MockTransport::default(), now).await;
        method.on_command(Command::LinkDown);

        // ACT
        method.on_command(Command::LinkUp);
        let silence = is_pending(method.next_event(now));

        // ASSERT
        assert!(silence);
        assert_eq!(method.socket.destinations().len(), 2);
    }

    #[tokio::test]
    async fn preferred_lifetime_expiry_keeps_address_deprecated() {
        // ARRANGE
        let now = Instant::now();
        let transport = MockTransport::default();
        transport.push_ra(ra(1800, Some((10, 1)), &[]), ROUTER);
        let mut method = new_method(transport);
        method.on_command(Command::Start);
        let _configure = method.next_event(now).await;
        let _route = method.next_event(now).await;
        let _bound = method.next_event(now).await;

        // ACT
        let silence = is_pending(method.next_event(at(now, 1)));

        // ASSERT
        assert!(silence);
        assert!(matches!(
            method.view(),
            Some(V6State::Slaac { addresses, .. }) if addresses == [(ADDRESS, 64)]
        ));
    }

    #[tokio::test]
    async fn router_expiry_removes_default_route() {
        // ARRANGE
        let now = Instant::now();
        let transport = MockTransport::default();
        transport.push_ra(ra(1, Some((100, 100)), &[]), ROUTER);
        let mut method = new_method(transport);
        method.on_command(Command::Start);
        let _configure = method.next_event(now).await;
        let _route = method.next_event(now).await;
        let _bound = method.next_event(now).await;

        // ACT
        let expired = method.next_event(at(now, 1)).await;

        // ASSERT
        assert!(matches!(
            expired,
            Event::Intent(Intent::RemoveDefaultRoute(IpAddr::V6(ROUTER)))
        ));
    }

    #[tokio::test]
    async fn rdnss_update_is_reflected_in_view() {
        // ARRANGE
        let now = Instant::now();
        let transport = MockTransport::default();
        transport.push_ra(ra(1800, Some((100, 100)), &[]), ROUTER);
        let mut method = new_method(transport);
        method.on_command(Command::Start);
        let _configure = method.next_event(now).await;
        let _route = method.next_event(now).await;
        let _bound = method.next_event(now).await;
        method
            .socket
            .push_ra(ra(1800, Some((100, 100)), &[(DNS, 3600)]), ROUTER);

        // ACT
        let _silence = is_pending(method.next_event(now));

        // ASSERT
        assert!(matches!(
            method.view(),
            Some(V6State::Slaac { dns, .. }) if dns == vec![DNS]
        ));
    }

    #[tokio::test]
    async fn bound_waits_for_the_router_deadline() {
        // ARRANGE
        let now = Instant::now();
        let transport = MockTransport::default();
        transport.push_ra(ra(1800, Some((7200, 3600)), &[]), ROUTER);
        let mut method = new_method(transport);

        // ACT
        method.on_command(Command::Start);
        let _configure = method.next_event(now).await;
        let _route = method.next_event(now).await;
        let _bound = method.next_event(now).await;
        let wake = method.next_wake(now);

        // ASSERT
        assert_eq!(wake, Some(at(now, 1800)));
        assert!(is_pending(method.next_event(now)));
    }

    #[tokio::test]
    async fn suspended_method_has_no_wake_deadline() {
        // ARRANGE
        let now = Instant::now();
        let mut method = bound(MockTransport::default(), now).await;

        // ACT
        method.on_command(Command::LinkDown);
        let wake = method.next_wake(now);

        // ASSERT
        assert_eq!(wake, None);
        assert!(is_pending(method.next_event(now)));
    }
}
