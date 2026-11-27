//! Sans-io SLAAC step, input dispatch, and effect pumping.

mod intent;
mod ra;
mod solicit;
pub(crate) mod state;
mod step;
mod timers;

use alloc::collections::VecDeque;
use std::time::{Instant, SystemTime};

use ip_method::{Event, Input, LinkInfo, Method, Step};
use netmodel::Ipv6Config;

use crate::{SOLICITATION_LEN, Slaac, View};

/// One queued step action that becomes an effect when pumped.
pub(crate) enum Action {
    Open,
    Close,
    SendSolicitation,
    Emit(Event),
}

impl Method for Slaac {
    type View = View;

    fn step(&mut self, now: Instant, _wall: SystemTime, input: Input<'_>) -> Step<'_> {
        step::apply(self, now, input);
        step::tick(self, now);

        step::pump(self, now)
    }

    fn next_wake(&self, now: Instant) -> Option<Instant> {
        timers::next_wake(self, now)
    }

    fn view(&self) -> Option<Self::View> {
        report(self)
    }
}

impl Slaac {
    /// Creates a SLAAC method for the given link and IPv6 configuration.
    #[must_use = "the method owns the provisioning state"]
    pub fn new(link: LinkInfo, config: Ipv6Config) -> Self {
        Self {
            fallback_dns: config.dns,
            link,
            open: false,
            running: false,
            solicited: false,
            solicitation_attempt: 0,
            solicitation_at: None,
            retry_at: None,
            address: None,
            router: None,
            dns_servers: Vec::new(),
            queue: VecDeque::new(),
            solicitation: [0_u8; SOLICITATION_LEN],
        }
    }

    pub(crate) fn warn(&mut self, message: String) {
        self.queue.push_back(Action::Emit(Event::Warning(message)));
    }
}

fn report(method: &Slaac) -> Option<View> {
    if method.address.is_none() && method.router.is_none() && method.dns_servers.is_empty() {
        return None;
    }

    Some(View {
        addresses: method
            .address
            .iter()
            .map(|address| (address.address, address.prefix_len))
            .collect(),
        routers: method.router.iter().map(|router| router.value).collect(),
        dns: method.dns_servers.iter().map(|dns| dns.value).collect(),
    })
}
