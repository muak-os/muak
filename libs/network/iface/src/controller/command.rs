//! Supervisor command and link transition handling.

use alloc::string::String;
use alloc::vec::Vec;

use ip_method::{Command as MethodCommand, Input as MethodInput};
use net_observe::journal::Journal;
use net_observe::{Event as ObserveEvent, Level};
use netmodel::{Family, MethodState};

use crate::method::{Replacement, V4Slot, V6Slot};
use crate::{Command, Controller, Effect};

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

pub(super) fn install(controller: &mut Controller, journal: &mut Journal) -> Effect<'static> {
    if let Some(replacement) = controller.deferred.take() {
        controller.v4 = replacement.v4;
        controller.v6 = replacement.v6;
        controller.snapshot.v4 = None;
        controller.snapshot.v4_state = MethodState::Stopped;
        controller.snapshot.v6 = None;
        controller.snapshot.v6_state = MethodState::Stopped;
        controller.pending.clear();
        for notice in &replacement.notices {
            journal.record(
                Level::Warn,
                ObserveEvent::Notice {
                    name: controller.link.name.clone(),
                    family: None,
                    text: notice.clone(),
                },
            );
        }
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

pub(super) fn link(controller: &mut Controller, info: &ip_method::LinkInfo) {
    let changed = info.state != controller.link.state;
    controller.snapshot.link = info.state;
    controller.link = info.clone();
    if !changed {
        return;
    }

    let up = info.state.has_carrier();
    if let Some(input) = link_input(controller, Family::V4, info, up) {
        controller.pending.push_back((Family::V4, input));
    }
    if let Some(input) = link_input(controller, Family::V6, info, up) {
        controller.pending.push_back((Family::V6, input));
    }
}

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

    broadcast(controller, MethodCommand::Stop);
}

fn replacement_notices(
    v4: Option<&netmodel::Ipv4Config>,
    v6: Option<&netmodel::Ipv6Config>,
) -> Vec<String> {
    let mut notices = Vec::new();

    if cfg!(not(feature = "ipv4"))
        && let Some(config) = v4
        && config.addresses.is_empty()
        && config.dhcp
    {
        notices.push(String::from(
            "DHCP requested but the ipv4 feature is disabled",
        ));
    }

    if cfg!(not(feature = "ipv6"))
        && let Some(config) = v6
        && config.addresses.is_empty()
        && config.autoconf
    {
        notices.push(String::from(
            "SLAAC requested but the ipv6 feature is disabled",
        ));
    }

    notices
}

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
