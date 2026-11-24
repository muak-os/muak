//! Stepping, method routing, and effect translation for the controller.

use std::time::{Instant, SystemTime};

use ip_method::{Effect as MethodEffect, Method, Step as MethodStep};

use super::{command, event};
use crate::{Controller, Effect, Family, Input, Step};

enum Transport<'a> {
    Open(ip_method::SocketSpec),
    Send {
        slot: ip_method::SocketSlot,
        frame: &'a [u8],
        dst: Option<core::net::SocketAddr>,
    },
    Close(ip_method::SocketSlot),
}

enum Reaction<'a> {
    Wait,
    Entropy,
    Transport(Transport<'a>),
    Event(ip_method::Event),
}

pub(super) fn dispatch<'a>(
    controller: &'a mut Controller,
    now: Instant,
    wall: SystemTime,
    input: Input<'_>,
) -> Step<'a> {
    match input {
        Input::Poll | Input::Timer => drain(controller, now, wall),
        Input::Method { family, input } => advance(controller, now, wall, family, input),
        Input::Command(command) => {
            command::apply(controller, command);
            drain(controller, now, wall)
        }
        Input::Link(info) => {
            command::link(controller, &info);
            if controller.v4.is_none() && controller.v6.is_none() {
                return Step::Effect(Effect::Publish);
            }
            drain(controller, now, wall)
        }
    }
}

pub(super) fn wake(controller: &Controller, now: Instant) -> Option<Instant> {
    if !controller.pending.is_empty() || controller.deferred.is_some() {
        return Some(now);
    }

    let v4 = controller
        .v4
        .as_ref()
        .and_then(|method| method.next_wake(now));
    let v6 = controller
        .v6
        .as_ref()
        .and_then(|method| method.next_wake(now));

    match (v4, v6) {
        (Some(v4), Some(v6)) => Some(v4.min(v6)),
        (Some(wake), None) | (None, Some(wake)) => Some(wake),
        (None, None) => None,
    }
}

fn drain(controller: &mut Controller, now: Instant, wall: SystemTime) -> Step<'_> {
    if let Some(effect) = pending_effect(controller, now, wall) {
        return effect;
    }
    if let Step::Effect(effect) = advance(controller, now, wall, Family::V4, ip_method::Input::Poll)
    {
        return Step::Effect(effect);
    }
    if let Step::Effect(effect) = advance(controller, now, wall, Family::V6, ip_method::Input::Poll)
    {
        return Step::Effect(effect);
    }

    if controller.deferred.is_some() {
        return Step::Effect(command::install(controller));
    }

    if controller.shutdown {
        controller.shutdown = false;
        controller.v4 = None;
        controller.v6 = None;

        return Step::Effect(Effect::Publish);
    }

    Step::Wait
}

fn pending_effect(controller: &mut Controller, now: Instant, wall: SystemTime) -> Option<Step<'_>> {
    let (family, input) = controller.pending.pop_front()?;
    let step = advance(controller, now, wall, family, input);

    match step {
        Step::Wait => pending_effect(controller, now, wall),
        effect @ Step::Effect(_) => Some(effect),
    }
}

fn advance<'a>(
    controller: &'a mut Controller,
    now: Instant,
    wall: SystemTime,
    family: Family,
    input: ip_method::Input<'_>,
) -> Step<'a> {
    let reaction = match family {
        Family::V4 => match controller.v4.as_mut() {
            Some(method) => react(method, now, wall, input),
            None => return Step::Wait,
        },
        Family::V6 => match controller.v6.as_mut() {
            Some(method) => react(method, now, wall, input),
            None => return Step::Wait,
        },
    };

    match reaction {
        Reaction::Wait => Step::Wait,
        Reaction::Entropy => Step::Effect(Effect::RequestEntropy { family }),
        Reaction::Transport(transport) => Step::Effect(translate(family, transport)),
        Reaction::Event(event) => Step::Effect(event::handle(controller, family, event)),
    }
}

fn react<'a, M: Method>(
    method: &'a mut M,
    now: Instant,
    wall: SystemTime,
    input: ip_method::Input<'_>,
) -> Reaction<'a> {
    match method.step(now, wall, input) {
        MethodStep::Wait => Reaction::Wait,
        MethodStep::Effect(MethodEffect::Emit(event)) => Reaction::Event(event),
        MethodStep::Effect(MethodEffect::RequestEntropy) => Reaction::Entropy,
        MethodStep::Effect(MethodEffect::OpenSocket(spec)) => {
            Reaction::Transport(Transport::Open(spec))
        }
        MethodStep::Effect(MethodEffect::Send { slot, frame, dst }) => {
            Reaction::Transport(Transport::Send { slot, frame, dst })
        }
        MethodStep::Effect(MethodEffect::CloseSocket { slot }) => {
            Reaction::Transport(Transport::Close(slot))
        }
    }
}

fn translate(family: Family, transport: Transport<'_>) -> Effect<'_> {
    match transport {
        Transport::Open(spec) => Effect::OpenSocket { family, spec },
        Transport::Send { slot, frame, dst } => Effect::Send {
            family,
            slot,
            frame,
            dst,
        },
        Transport::Close(slot) => Effect::CloseSocket { family, slot },
    }
}
