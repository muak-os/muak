//! Snapshot and effect updates from method lifecycle events.

use alloc::string::String;

use ip_method::{Event, Intent, State};
use net_observe::interface::MethodState as ObserveMethodState;
use net_observe::journal::Journal;
use net_observe::{Event as ObserveEvent, Level};

use super::observe::{family as observe_family, refresh};
use crate::{Controller, Effect, Family};

pub(super) const MAX_NOTICES: usize = 8;

pub(super) fn handle(
    controller: &mut Controller,
    family: Family,
    event: Event,
    journal: &mut Journal,
) -> Effect<'static> {
    match event {
        Event::Intent(intent) => {
            let effect = intent_effect(controller, intent);

            refresh(controller, family, journal);

            effect
        }
        Event::State(state) => {
            if state != current_state(controller, family) {
                journal.record(
                    Level::Info,
                    ObserveEvent::InterfaceState {
                        name: controller.link.name.clone(),
                        family: observe_family(family),
                        state: method_state(state),
                    },
                );
            }
            set_state(controller, family, state);
            refresh(controller, family, journal);

            Effect::Publish
        }
        Event::Warning(message) => {
            journal.record(Level::Warn, notice(controller, family, message.clone()));
            push_notice(controller, message);

            Effect::Publish
        }
        Event::Failed(failure) => {
            let text = failure.to_string();
            journal.record(Level::Error, notice(controller, family, text.clone()));
            push_notice(controller, text);
            set_state(controller, family, State::Degraded);
            refresh(controller, family, journal);

            Effect::Publish
        }
    }
}

fn notice(controller: &Controller, family: Family, text: String) -> ObserveEvent {
    ObserveEvent::Notice {
        name: controller.link.name.clone(),
        family: Some(observe_family(family)),
        text,
    }
}

fn push_notice(controller: &mut Controller, message: String) {
    if controller.snapshot.notices.len() >= MAX_NOTICES {
        controller.snapshot.notices.drain(..1);
    }
    controller.snapshot.notices.push(message);
}

fn intent_effect(controller: &Controller, intent: Intent) -> Effect<'static> {
    let link = controller.link.name.clone();

    match intent {
        Intent::Configure(address) => Effect::AddAddress {
            link,
            address: address.address,
            prefix_len: address.prefix_len,
        },
        Intent::Deconfigure(address) => Effect::RemoveAddress {
            link,
            address: address.address,
            prefix_len: address.prefix_len,
        },
        Intent::EnsureDefaultRoute(gateway) => Effect::EnsureRoute { link, gateway },
        Intent::RemoveDefaultRoute(gateway) => Effect::RemoveRoute { link, gateway },
    }
}

const fn current_state(controller: &Controller, family: Family) -> State {
    match family {
        Family::V4 => controller.snapshot.v4_state,
        Family::V6 => controller.snapshot.v6_state,
    }
}

const fn set_state(controller: &mut Controller, family: Family, state: State) {
    match family {
        Family::V4 => controller.snapshot.v4_state = state,
        Family::V6 => controller.snapshot.v6_state = state,
    }
}

const fn method_state(state: State) -> ObserveMethodState {
    match state {
        State::Acquiring => ObserveMethodState::Acquiring,
        State::Bound => ObserveMethodState::Bound,
        State::Degraded => ObserveMethodState::Degraded,
        State::Stopped => ObserveMethodState::Stopped,
    }
}
