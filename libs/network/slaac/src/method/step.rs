//! Input dispatch, timer handling, and effect pumping for one step.

use core::net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV6};
use std::io;
use std::time::Instant;

use ip_method::{
    Command, Effect, Event, Failure, Input, LinkInfo, SocketKind, SocketSlot, SocketSpec, State,
    Step,
};
use ndp::ALL_ROUTERS;
use netmodel::LinkState;

use super::{Action, intent, ra, solicit, timers};
use crate::{MAX_RTR_SOLICITATIONS, RTR_SOLICITATION_INTERVAL, Slaac};

pub(crate) fn apply(method: &mut Slaac, now: Instant, input: Input<'_>) {
    match input {
        Input::Command(command) => handle_command(method, now, command),
        Input::SocketOpened {
            slot: SocketSlot::Primary,
        } => opened(method, now),
        Input::SocketClosed {
            slot: SocketSlot::Primary,
        } => closed(method, now),
        Input::Frame {
            slot: SocketSlot::Primary,
            bytes,
            source,
        } => frame(method, now, bytes, source),
        Input::SocketError {
            slot: SocketSlot::Primary,
            error,
        } => handle_error(method, now, error),
        Input::Link(link) => update_link(method, now, link),
        Input::Poll
        | Input::SendCompleted { .. }
        | Input::Entropy(_)
        | Input::SocketOpened { .. }
        | Input::SocketClosed { .. }
        | Input::Frame { .. }
        | Input::SocketError { .. }
        | Input::Timer => {}
    }
}

pub(crate) fn tick(method: &mut Slaac, now: Instant) {
    if !method.running {
        return;
    }
    let retry_due = method.retry_at.is_some_and(|at| at <= now);
    if retry_due {
        method.retry_at = None;
    }
    if !method.open {
        reopen(method, retry_due);
        return;
    }
    if method.solicited {
        expire(method, now);
        return;
    }
    if backoff(method, now) {
        return;
    }
    if method.solicitation_attempt >= MAX_RTR_SOLICITATIONS {
        solicit::exhaust(method);
        return;
    }
    if method.solicitation_at.is_none_or(|at| at <= now) {
        solicit::schedule(method, now);
    }
}

pub(crate) fn pump(method: &mut Slaac, now: Instant) -> Step<'_> {
    let Some(action) = method.queue.pop_front() else {
        return Step::Wait;
    };

    match action {
        Action::Open => Step::Effect(Effect::OpenSocket(spec(method))),
        Action::Close => Step::Effect(Effect::CloseSocket {
            slot: SocketSlot::Primary,
        }),
        Action::Emit(event) => Step::Effect(Effect::Emit(event)),
        Action::SendSolicitation if method.open && !backoff(method, now) => {
            send_solicitation(method)
        }
        Action::SendSolicitation => Step::Wait,
    }
}

fn handle_command(method: &mut Slaac, now: Instant, command: Command) {
    match command {
        Command::Start | Command::LinkUp => solicit::start(method, now),
        Command::Reconcile => intent::reapply(method, now),
        Command::LinkDown => solicit::suspend(method),
        Command::Stop => solicit::stop(method),
    }
}

fn opened(method: &mut Slaac, now: Instant) {
    method.open = true;
    if !method.running || method.solicited || backoff(method, now) {
        return;
    }
    method
        .queue
        .push_back(Action::Emit(Event::State(State::Acquiring)));
    if method.solicitation_at.is_none() {
        solicit::schedule(method, now);
    }
}

fn closed(method: &mut Slaac, now: Instant) {
    method.open = false;
    if !method.running {
        return;
    }
    method.retry_at = now.checked_add(RTR_SOLICITATION_INTERVAL);
    method.warn(String::from(
        "primary ICMPv6 socket closed unexpectedly, reopening after backoff",
    ));
}

fn frame(method: &mut Slaac, now: Instant, bytes: &[u8], source: Option<IpAddr>) {
    if !method.running || !method.open || backoff(method, now) {
        return;
    }
    let Some(IpAddr::V6(source)) = source else {
        return;
    };
    if !link_local(source) {
        return;
    }
    ra::receive(method, source, bytes, now);
}

fn handle_error(method: &mut Slaac, now: Instant, error: io::Error) {
    if !method.running {
        return;
    }
    method.retry_at = now.checked_add(RTR_SOLICITATION_INTERVAL);
    if matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
    ) {
        method.warn(format!("transient ICMPv6 socket error: {error}, retrying"));
    } else {
        method
            .queue
            .push_back(Action::Emit(Event::Failed(Failure::Io(error))));
    }
}

fn update_link(method: &mut Slaac, now: Instant, link: LinkInfo) {
    let running = method.running;
    method.link = link;
    match method.link.state {
        LinkState::Down => solicit::suspend(method),
        LinkState::Up if running => solicit::start(method, now),
        LinkState::Up | LinkState::NoCarrier | LinkState::Dormant => {}
    }
}

fn backoff(method: &Slaac, now: Instant) -> bool {
    method.retry_at.is_some_and(|at| now < at)
}

fn reopen(method: &mut Slaac, retry_due: bool) {
    if retry_due {
        method.queue.push_back(Action::Open);
    }
}

fn expire(method: &mut Slaac, now: Instant) {
    if timers::due(method, now) {
        timers::handle_expiration(method, now);
    }
}

fn send_solicitation(method: &mut Slaac) -> Step<'_> {
    let Ok(frame) = ndp::build::router_solicitation(
        Ipv6Addr::UNSPECIFIED,
        ALL_ROUTERS,
        None,
        &mut method.solicitation,
    ) else {
        method.warn(String::from("could not build a Router Solicitation"));
        return Step::Wait;
    };
    let len = frame.len();
    let Some(frame) = method.solicitation.get(..len) else {
        method.warn(String::from("Router Solicitation buffer is too small"));
        return Step::Wait;
    };

    Step::Effect(Effect::Send {
        slot: SocketSlot::Primary,
        frame,
        dst: Some(SocketAddr::V6(SocketAddrV6::new(ALL_ROUTERS, 0, 0, 0))),
    })
}

fn spec(method: &Slaac) -> SocketSpec {
    SocketSpec {
        slot: SocketSlot::Primary,
        kind: SocketKind::RawIcmpv6,
        link: method.link.name.clone(),
        bind: None,
        peer: None,
    }
}

const fn link_local(address: Ipv6Addr) -> bool {
    address.segments()[0] & 0xffc0 == 0xfe80
}
