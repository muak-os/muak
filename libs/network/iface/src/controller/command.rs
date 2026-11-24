//! Supervisor command and link transition handling.

use alloc::string::String;
use alloc::vec::Vec;

use ip_method::{Command as MethodCommand, Input as MethodInput, State};

use crate::method::{Replacement, V4Slot, V6Slot};
use crate::{Command, Controller, Effect, Family};

// Applies a supervisor command to the controller and its methods.
pub(super) fn apply(controller: &mut Controller, command: Command) {
    match command {
        Command::Configure { v4, v6 } => configure(controller, v4, v6),
        Command::Reconcile => broadcast(controller, MethodCommand::Reconcile),
        Command::LinkUp => broadcast(controller, MethodCommand::LinkUp),
        Command::LinkDown => broadcast(controller, MethodCommand::LinkDown),
        Command::Shutdown => {
            broadcast(controller, MethodCommand::Stop);
            controller.shutdown = true;
        }
    }
}

// Installs the deferred replacement once the outgoing methods stopped.
pub(super) fn install(controller: &mut Controller) -> Effect<'static> {
    if let Some(replacement) = controller.deferred.take() {
        controller.v4 = replacement.v4;
        controller.v6 = replacement.v6;
        controller.snapshot.v4 = None;
        controller.snapshot.v4_state = State::Stopped;
        controller.snapshot.v6 = None;
        controller.snapshot.v6_state = State::Stopped;
        controller.pending.clear();
        controller.snapshot.notices.clear();
        controller.snapshot.notices.extend(replacement.notices);
        controller.shutdown = false;

        if controller.v4.is_some() {
            let start = MethodInput::Command(MethodCommand::Start);
            controller.pending.push_back((Family::V4, start));
        }
        if controller.v6.is_some() {
            let start = MethodInput::Command(MethodCommand::Start);
            controller.pending.push_back((Family::V6, start));
        }
    }

    Effect::Publish
}

// Updates the link and forwards the transition to the family methods.
pub(super) fn link(controller: &mut Controller, info: &ip_method::LinkInfo) {
    let changed = info.state != controller.link.state;
    controller.snapshot.link = info.state;
    controller.link = info.clone();
    if !changed {
        return;
    }

    let up = matches!(info.state, netmodel::LinkState::Up);
    if let Some(input) = link_input(controller, Family::V4, info, up) {
        controller.pending.push_back((Family::V4, input));
    }
    if let Some(input) = link_input(controller, Family::V6, info, up) {
        controller.pending.push_back((Family::V6, input));
    }
}

// Builds the replacement methods and stops the outgoing ones.
fn configure(
    controller: &mut Controller,
    v4: Option<netmodel::Ipv4Config>,
    v6: Option<netmodel::Ipv6Config>,
) {
    controller.pending.clear();
    controller.snapshot.notices.clear();
    controller.shutdown = false;

    let notices = replacement_notices(v4.as_ref(), v6.as_ref());

    let v4_slot = v4.and_then(|config| V4Slot::new(&controller.link, config));
    let v6_slot = v6.and_then(|config| V6Slot::new(&controller.link, config));

    controller.deferred = Some(Replacement {
        v4: v4_slot,
        v6: v6_slot,
        notices,
    });

    // Stop the outgoing methods; the swap happens once both return `Wait`.
    broadcast(controller, MethodCommand::Stop);
}

// Explains configurations whose enabling feature is not compiled in.
#[cfg(any(not(feature = "ipv4"), not(feature = "ipv6")))]
fn replacement_notices(
    v4: Option<&netmodel::Ipv4Config>,
    v6: Option<&netmodel::Ipv6Config>,
) -> Vec<String> {
    // One family has no notice branch in single-feature builds.
    let _ = v4;
    let _ = v6;

    let mut notices = Vec::new();
    #[cfg(not(feature = "ipv4"))]
    if let Some(notice) = v4.and_then(|config| {
        (config.addresses.is_empty() && config.dhcp)
            .then(|| String::from("DHCP requested but the ipv4 feature is disabled"))
    }) {
        notices.push(notice);
    }
    #[cfg(not(feature = "ipv6"))]
    if let Some(notice) = v6.and_then(|config| {
        (config.addresses.is_empty() && config.autoconf)
            .then(|| String::from("SLAAC requested but the ipv6 feature is disabled"))
    }) {
        notices.push(notice);
    }
    notices
}

// With both features compiled in no configuration can be unsupported.
#[cfg(all(feature = "ipv4", feature = "ipv6"))]
const fn replacement_notices(
    _v4: Option<&netmodel::Ipv4Config>,
    _v6: Option<&netmodel::Ipv6Config>,
) -> Vec<String> {
    Vec::new()
}

// Queues the same method command for every running family method.
fn broadcast(controller: &mut Controller, command: MethodCommand) {
    if controller.v4.is_some() {
        controller
            .pending
            .push_back((Family::V4, MethodInput::Command(command)));
    }
    if controller.v6.is_some() {
        controller
            .pending
            .push_back((Family::V6, MethodInput::Command(command)));
    }
}

// Builds the link input the family method expects for a transition.
fn link_input(
    controller: &Controller,
    family: Family,
    info: &ip_method::LinkInfo,
    up: bool,
) -> Option<MethodInput<'static>> {
    let command = if up {
        MethodCommand::LinkUp
    } else {
        MethodCommand::LinkDown
    };

    match family {
        Family::V4 => {
            let slot = controller.v4.as_ref()?;
            Some(transition_input(slot.is_dhcp(), info, command))
        }
        Family::V6 => {
            let slot = controller.v6.as_ref()?;
            Some(transition_input(slot.is_stateless(), info, command))
        }
    }
}

// Builds the input a dynamic method takes on a link transition.
fn transition_input(
    dynamic: bool,
    info: &ip_method::LinkInfo,
    command: MethodCommand,
) -> MethodInput<'static> {
    if dynamic {
        MethodInput::Link(info.clone())
    } else {
        MethodInput::Command(command)
    }
}
