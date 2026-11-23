//! Router solicitation, initial advertisement handling, and monitoring.

use alloc::format;
use core::net::{IpAddr, Ipv6Addr};
use std::time::Instant;

use addrconf::{Event, Failure, Intent, State, Step};

use super::ra::process_unsolicited_ra;
use super::state::{ManagedAddress, ManagedRouter};
use super::transport::Transport;
use super::{MAX_RTR_SOLICITATIONS, RTR_SOLICITATION_INTERVAL, Slaac, intent, timers};
use crate::address::generate;
use crate::icmpv6::build::router_solicitation;
use crate::icmpv6::parse::router_advertisement;
use crate::icmpv6::types::RouterAdvertisement;
use crate::icmpv6::{ALL_ROUTERS, ROUTER_ADVERTISEMENT};

pub(super) async fn initial<T: Transport>(method: &mut Slaac<T>, now: Instant) -> Step {
    if method.solicitation_attempt >= MAX_RTR_SOLICITATIONS {
        return exhaust(method);
    }

    if method.solicitation_at.is_none_or(|at| now >= at) {
        return solicit(method, now).await;
    }

    receive(method, now).await
}

pub(super) async fn monitor<T: Transport>(method: &mut Slaac<T>, now: Instant) -> Step {
    if timers::due(method, now) {
        timers::handle_expiration(method, now);
        return Step::Advance;
    }

    if let Some(at) = method.retry_at {
        if now < at {
            return Step::Wait;
        }
        method.retry_at = None;
    }

    match try_recv_ra(method).await {
        Ok(Some(ra)) => {
            process_unsolicited_ra(method, &ra, now);
            Step::Advance
        }
        Ok(None) => Step::Advance,
        Err(error) => {
            method.retry_at = now.checked_add(RTR_SOLICITATION_INTERVAL);
            method.queue.push_back(Event::Failed(Failure::Io(error)));
            Step::Advance
        }
    }
}

pub(super) fn process_initial_ra<T: Transport>(
    method: &mut Slaac<T>,
    ra: &RouterAdvertisement,
    now: Instant,
) -> bool {
    let Some(prefix) = ra
        .prefixes
        .iter()
        .find(|prefix| prefix.autonomous && prefix.prefix_len <= 64)
    else {
        return false;
    };

    let Some(address) = generate(prefix.prefix, prefix.prefix_len, &method.mac) else {
        return false;
    };

    method.address = Some(ManagedAddress::new(
        address,
        prefix.prefix_len,
        ra.source,
        prefix.valid_lifetime,
        prefix.preferred_lifetime,
        now,
    ));
    method.router = Some(ManagedRouter::new(
        ra.source,
        u64::from(ra.router_lifetime),
        now,
    ));
    method.dns_servers = intent::resolve_dns(method, ra, now);

    method
        .queue
        .push_back(Event::Intent(intent::configure(address, prefix.prefix_len)));
    if ra.router_lifetime != 0 {
        method
            .queue
            .push_back(Event::Intent(Intent::EnsureDefaultRoute(IpAddr::V6(
                ra.source,
            ))));
    }
    method.queue.push_back(Event::State(State::Bound));

    true
}

async fn solicit<T: Transport>(method: &mut Slaac<T>, now: Instant) -> Step {
    let packet = router_solicitation(Ipv6Addr::UNSPECIFIED, &method.mac);

    method.solicitation_attempt = method.solicitation_attempt.saturating_add(1);
    method.solicitation_at = now.checked_add(RTR_SOLICITATION_INTERVAL);

    if let Err(error) = method.socket.send_to(ALL_ROUTERS, &packet).await {
        method.queue.push_back(Event::Warning(format!(
            "could not send RS on {}: {error}",
            method.interface
        )));
        return Step::Advance;
    }

    receive(method, now).await
}

async fn receive<T: Transport>(method: &mut Slaac<T>, now: Instant) -> Step {
    match try_recv_ra(method).await {
        Ok(Some(ra)) => settle(method, &ra, now),
        Ok(None) => Step::Advance,
        Err(_error) => Step::Wait,
    }
}

fn settle<T: Transport>(method: &mut Slaac<T>, ra: &RouterAdvertisement, now: Instant) -> Step {
    method.solicited = true;
    method.solicitation_at = None;

    if !process_initial_ra(method, ra, now) {
        method.queue.push_back(Event::Warning(format!(
            "no usable autonomous prefix on {}, continuing to listen",
            method.interface
        )));
    }

    Step::Advance
}

fn exhaust<T: Transport>(method: &mut Slaac<T>) -> Step {
    method.solicited = true;
    method.solicitation_at = None;
    method.queue.push_back(Event::Warning(format!(
        "no Router Advertisement on {} after {MAX_RTR_SOLICITATIONS} attempts, continuing to listen",
        method.interface
    )));

    Step::Advance
}

async fn try_recv_ra<T: Transport>(
    method: &Slaac<T>,
) -> std::io::Result<Option<RouterAdvertisement>> {
    let mut buf = [0_u8; 1500];

    let (len, source) = method.socket.recv_from(&mut buf).await?;

    if buf.first().copied().unwrap_or(0) != ROUTER_ADVERTISEMENT {
        return Ok(None);
    }

    let source = source.unwrap_or(Ipv6Addr::UNSPECIFIED);
    let data = buf.get(..len).unwrap_or_default();
    let ra = router_advertisement(data, source).map_err(std::io::Error::other)?;

    Ok(Some(ra))
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use alloc::vec::Vec;
    use core::future::Future;
    use core::net::Ipv6Addr;
    use core::time::Duration;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Instant;

    use addrconf::v6::Config;
    use addrconf::{Command, Event, Failure, Method as _};

    use super::Slaac;
    use crate::method::tests::{is_pending, next_event};
    use crate::method::transport::Transport;

    const MAC: [u8; 6] = [0x00, 0x1a, 0x2b, 0x3c, 0x4d, 0x5e];

    #[derive(Default)]
    struct FailingTransport {
        receives: AtomicU32,
    }

    impl FailingTransport {
        fn receives(&self) -> u32 {
            self.receives.load(Ordering::Relaxed)
        }
    }

    impl Transport for FailingTransport {
        fn recv_from(
            &self,
            _buf: &mut [u8],
        ) -> impl Future<Output = std::io::Result<(usize, Option<Ipv6Addr>)>> + Send {
            self.receives.fetch_add(1, Ordering::Relaxed);
            core::future::ready(Err(std::io::Error::from(std::io::ErrorKind::NotConnected)))
        }

        fn send_to(
            &self,
            _dst: Ipv6Addr,
            _payload: &[u8],
        ) -> impl Future<Output = std::io::Result<usize>> + Send {
            core::future::ready(Ok(0))
        }
    }

    fn monitoring(transport: FailingTransport) -> Slaac<FailingTransport> {
        let config = Config {
            addresses: Vec::new(),
            gateway: None,
            dns: Vec::new(),
            autoconf: true,
        };
        let mut method = Slaac::new(config, String::from("eth0"), MAC, transport);
        method.running = true;
        method.solicited = true;

        method
    }

    fn at(now: Instant, secs: u64) -> Instant {
        now.checked_add(Duration::from_secs(secs))
            .expect("representable instant")
    }

    #[test]
    fn receive_error_is_paced_by_the_back_off() {
        // ARRANGE
        let now = Instant::now();
        let mut method = monitoring(FailingTransport::default());

        // ACT
        let failed = next_event(&mut method, now);
        let wake = method.next_wake(now);
        let receives_after_failure = method.socket.receives();
        let repeat = is_pending(method.next_event(now));

        // ASSERT
        assert!(matches!(failed, Event::Failed(Failure::Io(_))));
        assert_eq!(wake, Some(at(now, 4)));
        assert_eq!(receives_after_failure, 1);
        assert!(repeat);
        assert_eq!(method.socket.receives(), 1);
    }

    #[test]
    fn receive_error_retries_once_after_the_interval() {
        // ARRANGE
        let now = Instant::now();
        let mut method = monitoring(FailingTransport::default());
        let _failed = next_event(&mut method, now);

        // ACT
        let retried = next_event(&mut method, at(now, 4));
        let wake = method.next_wake(at(now, 4));

        // ASSERT
        assert!(matches!(retried, Event::Failed(Failure::Io(_))));
        assert_eq!(method.socket.receives(), 2);
        assert_eq!(wake, Some(at(now, 8)));
    }

    #[test]
    fn stop_and_resolicit_clear_the_back_off() {
        // ARRANGE
        let now = Instant::now();
        let mut method = monitoring(FailingTransport::default());
        let _failed = next_event(&mut method, now);

        // ACT
        method.on_command(Command::Stop);
        let stopped = method.next_wake(now);
        method.on_command(Command::Start);
        let resolicited = method.next_wake(at(now, 1));

        // ASSERT
        assert_eq!(stopped, None);
        assert_eq!(resolicited, Some(at(now, 5)));
    }
}
