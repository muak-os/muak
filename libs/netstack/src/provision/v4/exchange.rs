//! `DHCPv4` exchanges driven by the lease state machine.

use alloc::format;
use core::net::Ipv4Addr;
use std::time::Instant;

use dhcp_client::Failure as DhcpFailure;
use dhcp_client::lease::Lease;

use super::dhcp::Phase;
use super::{Dhcp, Exchange, retry};
use crate::provision::{Event, Step};

pub(super) async fn acquire<E: Exchange>(method: &mut Dhcp<E>, now: Instant) -> Step {
    if let Some(at) = method.retry_at {
        if now < at {
            return Step::Wait;
        }
        // The deadline elapsed: clear it before awaiting I/O so the controller
        // never reads a past wake instant while the exchange is in flight.
        method.retry_at = None;
    }

    method.attempts = method.attempts.saturating_add(1);
    let result = method.exchange.acquire(&method.interface, method.mac).await;

    match retry::classify(method.attempts, result) {
        retry::Outcome::Acquired(lease) => {
            method.bind(now, lease);
            Step::Advance
        }
        retry::Outcome::Retry(delay) => {
            method.retry_at = now.checked_add(delay);
            // The warning also publishes the retry: the controller rereads the
            // wake deadline only after it has dispatched an event.
            method.queue.push_back(Event::Warning(format!(
                "DHCP acquire failed on {} after attempt {}, retrying",
                method.interface, method.attempts
            )));
            Step::Advance
        }
        retry::Outcome::Failed(error) => {
            method.attempts = 0;
            method.retry_at = None;
            method.degrade(&error);
            Step::Advance
        }
    }
}

pub(super) async fn renew<E: Exchange>(method: &mut Dhcp<E>, now: Instant) -> Step {
    if method.deadlines.expired(now) {
        method.expire();
        return Step::Advance;
    }

    let Some((server, assigned)) = method.lease.as_ref().map(endpoints) else {
        method.begin_acquire();
        return Step::Advance;
    };

    match method
        .exchange
        .renew(&method.interface, method.mac, server, assigned)
        .await
    {
        Ok(lease) => {
            method.bind(now, lease);
            Step::Advance
        }
        Err(DhcpFailure::Nak) => {
            method.begin_acquire();
            Step::Advance
        }
        Err(error) => {
            method.queue.push_back(Event::Warning(format!(
                "DHCP renew failed on {}: {error}",
                method.interface
            )));
            method.deadlines.skip_renew();
            method.phase = Phase::Bound;
            Step::Advance
        }
    }
}

pub(super) async fn rebind<E: Exchange>(method: &mut Dhcp<E>, now: Instant) -> Step {
    if method.deadlines.expired(now) {
        method.expire();
        return Step::Advance;
    }

    let Some((server, assigned)) = method.lease.as_ref().map(endpoints) else {
        method.begin_acquire();
        return Step::Advance;
    };

    match method
        .exchange
        .rebind(&method.interface, method.mac, server, assigned)
        .await
    {
        Ok(lease) => {
            method.bind(now, lease);
            Step::Advance
        }
        Err(DhcpFailure::Nak) => {
            method.begin_acquire();
            Step::Advance
        }
        Err(error) => {
            method.queue.push_back(Event::Warning(format!(
                "DHCP rebind failed on {}: {error}",
                method.interface
            )));
            method.deadlines.wait_expiry();
            method.phase = Phase::Bound;
            Step::Advance
        }
    }
}

const fn endpoints(lease: &Lease) -> (Ipv4Addr, Ipv4Addr) {
    (lease.server_ip, lease.assigned_ip)
}
