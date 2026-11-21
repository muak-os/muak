//! Router solicitation, initial advertisement handling, and monitoring.

use alloc::format;
use alloc::string::String;
use core::net::{IpAddr, Ipv6Addr};
use core::time::Duration;

use ::slaac::address::generate;
use ::slaac::icmpv6::build::router_solicitation;
use ::slaac::icmpv6::parse::router_advertisement;
use ::slaac::icmpv6::types::RouterAdvertisement;
use ::slaac::icmpv6::{ALL_ROUTERS, ROUTER_ADVERTISEMENT};
use tokio::time::{Instant, sleep, timeout};

use super::ra::process_unsolicited_ra;
use super::state::{ManagedAddress, ManagedRouter};
use super::{MAX_RTR_SOLICITATIONS, RTR_SOLICITATION_INTERVAL, Slaac, configure, resolve_dns};
use crate::provision::{Event, Failure, Intent, State};
use crate::transport::icmpv6::Transport;

pub(super) async fn initial<T: Transport>(method: &mut Slaac<T>) -> Result<(), String> {
    let packet = router_solicitation(Ipv6Addr::UNSPECIFIED, &method.mac);
    let mut last_error = None;

    while method.solicitation_attempt < MAX_RTR_SOLICITATIONS {
        method.solicitation_attempt = method.solicitation_attempt.saturating_add(1);
        let attempt = method.solicitation_attempt;

        if let Err(error) = method.socket.send_to(ALL_ROUTERS, &packet).await {
            last_error = Some(error);
            pause_before_retry(attempt).await;
            continue;
        }

        last_error = None;

        if let Some(ra) = wait_for_ra(method, RTR_SOLICITATION_INTERVAL).await {
            return process_initial(method, &ra);
        }
    }

    Err(match last_error {
        Some(error) => format!(
            "could not send RS on {} after {MAX_RTR_SOLICITATIONS} attempts: {error}",
            method.interface
        ),
        None => format!(
            "no Router Advertisement on {} after {MAX_RTR_SOLICITATIONS} attempts",
            method.interface
        ),
    })
}

fn process_initial<T: Transport>(
    method: &mut Slaac<T>,
    ra: &RouterAdvertisement,
) -> Result<(), String> {
    if process_initial_ra(method, ra) {
        Ok(())
    } else {
        Err(format!(
            "no usable autonomous prefix on {}",
            method.interface
        ))
    }
}

pub(super) async fn monitor<T: Transport + Send>(method: &mut Slaac<T>) -> Event {
    loop {
        if let Some(event) = method.queue.pop_front() {
            return event;
        }

        let deadline = super::timers::next_deadline(method);
        let remaining = deadline.saturating_duration_since(Instant::now());

        match timeout(remaining, try_recv_ra(method)).await {
            Ok(Ok(Some(ra))) => process_unsolicited_ra(method, &ra),
            Ok(Ok(None)) => {}
            Ok(Err(error)) => {
                sleep(RTR_SOLICITATION_INTERVAL).await;
                return Event::Failed(Failure::Io(error));
            }
            Err(_elapsed) => super::timers::handle_expiration(method),
        }
    }
}

pub(super) fn process_initial_ra<T: Transport>(
    method: &mut Slaac<T>,
    ra: &RouterAdvertisement,
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
    ));
    method.router = Some(ManagedRouter::new(ra.source, u64::from(ra.router_lifetime)));
    method.dns_servers = resolve_dns(method, ra);

    method
        .queue
        .push_back(Event::Intent(configure(address, prefix.prefix_len)));
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

async fn pause_before_retry(attempt: u32) {
    if attempt < MAX_RTR_SOLICITATIONS {
        sleep(RTR_SOLICITATION_INTERVAL).await;
    }
}

async fn wait_for_ra<T: Transport>(
    method: &Slaac<T>,
    wait_timeout: Duration,
) -> Option<RouterAdvertisement> {
    let deadline = Instant::now()
        .checked_add(wait_timeout)
        .unwrap_or_else(Instant::now);

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }

        match timeout(remaining, try_recv_ra(method)).await {
            Ok(Ok(Some(ra))) => return Some(ra),
            Ok(Ok(None)) => {}
            Ok(Err(_)) | Err(_) => return None,
        }
    }
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
