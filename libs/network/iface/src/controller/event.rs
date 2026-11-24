//! Snapshot and effect updates from method lifecycle events.

use alloc::string::String;

use ip_method::{Event, Intent, Method, State};

use crate::{Controller, Effect, Family};

pub(super) const MAX_NOTICES: usize = 8;

pub(super) fn handle(controller: &mut Controller, family: Family, event: Event) -> Effect<'static> {
    match event {
        Event::Intent(intent) => {
            let effect = intent_effect(controller, intent);

            refresh(controller, family);

            effect
        }
        Event::State(state) => {
            set_state(controller, family, state);
            refresh(controller, family);

            Effect::Publish
        }
        Event::Warning(message) => {
            push_notice(controller, message);

            Effect::Publish
        }
        Event::Failed(failure) => {
            push_notice(controller, failure.to_string());
            set_state(controller, family, State::Degraded);
            refresh(controller, family);

            Effect::Publish
        }
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

const fn set_state(controller: &mut Controller, family: Family, state: State) {
    match family {
        Family::V4 => controller.snapshot.v4_state = state,
        Family::V6 => controller.snapshot.v6_state = state,
    }
}

fn refresh(controller: &mut Controller, family: Family) {
    match family {
        Family::V4 => {
            let view = controller.v4.as_ref().and_then(Method::view);
            controller.snapshot.v4 = view;
        }
        Family::V6 => {
            let view = controller.v6.as_ref().and_then(Method::view);
            controller.snapshot.v6 = view;
        }
    }
}
