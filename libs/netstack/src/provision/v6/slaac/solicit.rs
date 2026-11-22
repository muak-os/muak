//! Router solicitation, initial advertisement handling, and monitoring.

use alloc::format;
use core::net::{IpAddr, Ipv6Addr};
use std::time::Instant;

use ::slaac::address::generate;
use ::slaac::icmpv6::build::router_solicitation;
use ::slaac::icmpv6::parse::router_advertisement;
use ::slaac::icmpv6::types::RouterAdvertisement;
use ::slaac::icmpv6::{ALL_ROUTERS, ROUTER_ADVERTISEMENT};

use super::ra::process_unsolicited_ra;
use super::state::{ManagedAddress, ManagedRouter};
use super::{MAX_RTR_SOLICITATIONS, RTR_SOLICITATION_INTERVAL, Slaac, intent, timers};
use crate::provision::{Event, Failure, Intent, State, Step};
use crate::transport::icmpv6::Transport;

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

    match try_recv_ra(method).await {
        Ok(Some(ra)) => {
            process_unsolicited_ra(method, &ra, now);
            Step::Advance
        }
        Ok(None) => Step::Advance,
        Err(error) => {
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

// Sends one solicitation, arms the next one and processes the reply it triggers.
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

// Waits for one inbound packet. A receive error keeps the solicitation deadline
// armed, so the next attempt is paced by the controller.
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
