//! `ip_method::Method` implementation of the `DHCPv4` method.

use std::time::{Instant, SystemTime};

use ip_method::{Input, Method, Step};

use crate::timer::Deadlines;
use crate::{Dhcp, Phase, View, effects, exchange, recv, state};

impl Method for Dhcp {
    type View = View;

    fn next_wake(&self, now: Instant) -> Option<Instant> {
        let deadline = match self.phase {
            Phase::Acquiring => self.retry_at.or_else(|| self.exchange.deadline()),
            Phase::Bound => bound_wake(self, now),
            Phase::Renewing | Phase::Rebinding => self.exchange.deadline(),
            Phase::Idle | Phase::Degraded | Phase::Stopped => None,
        };

        deadline.filter(|at| *at > now)
    }

    fn step(&mut self, now: Instant, wall: SystemTime, input: Input<'_>) -> Step<'_> {
        state::advance(self, now);
        consume(self, now, wall, input);
        state::advance(self, now);

        effects::emit(self, now)
    }

    fn view(&self) -> Option<Self::View> {
        Some(View {
            lease: self.lease.clone()?,
            phase: self.phase,
        })
    }
}

fn bound_wake(method: &Dhcp, now: Instant) -> Option<Instant> {
    if let Some(lease) = method.lease.as_ref() {
        let anchor = method.bound_at.unwrap_or(now);
        return method
            .deadlines
            .next()
            .or_else(|| Deadlines::preview(anchor, lease));
    }

    method.deadlines.next()
}

fn consume(method: &mut Dhcp, now: Instant, wall: SystemTime, input: Input<'_>) {
    match input {
        Input::Poll | Input::Timer | Input::SendCompleted { .. } => {}
        Input::Command(command) => state::command(method, command),
        Input::Link(link) => state::on_link(method, link),
        Input::SocketOpened { slot } => exchange::opened(method, slot),
        Input::SocketClosed { slot } => exchange::closed(method, slot),
        Input::SocketError { slot, .. } => exchange::on_socket_error(method, now, slot),
        Input::Entropy(xid) => exchange::on_entropy(method, xid),
        Input::Frame {
            slot,
            bytes,
            source,
        } => recv::frame(method, now, wall, slot, bytes, source),
    }
}
