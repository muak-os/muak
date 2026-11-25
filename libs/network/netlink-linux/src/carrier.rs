//! Administrative and carrier state derivation from link flags and attributes.

use netlink_packet_route::link::{LinkAttribute, LinkFlags, LinkMessage, State};
use netmodel::{AdminState, CarrierState, LinkState};

pub(crate) fn parts(message: &LinkMessage) -> (AdminState, CarrierState) {
    let admin = admin_state(message.header.flags);
    let carrier = if admin.is_up() {
        resolve(message)
    } else {
        CarrierState::Unknown
    };

    (admin, carrier)
}

pub(crate) fn state(message: &LinkMessage) -> LinkState {
    let (admin, carrier) = parts(message);

    LinkState::from_parts(admin, carrier)
}

const fn admin_state(flags: LinkFlags) -> AdminState {
    if flags.contains(LinkFlags::Up) {
        AdminState::Up
    } else {
        AdminState::Down
    }
}

fn resolve(message: &LinkMessage) -> CarrierState {
    match oper_state(message) {
        Some(State::Up) => CarrierState::Up,
        Some(State::Dormant) => CarrierState::Dormant,
        Some(State::NotPresent | State::Down | State::LowerLayerDown | State::Testing) => {
            CarrierState::Down
        }
        _ => fallback(message),
    }
}

fn oper_state(message: &LinkMessage) -> Option<State> {
    message.attributes.iter().find_map(|attribute| {
        let LinkAttribute::OperState(state) = *attribute else {
            return None;
        };

        Some(state)
    })
}

fn fallback(message: &LinkMessage) -> CarrierState {
    let flags = message.header.flags;
    if flags.contains(LinkFlags::Dormant) {
        return CarrierState::Dormant;
    }

    match attribute(message) {
        Some(1) => return CarrierState::Up,
        Some(0) => return CarrierState::Down,
        Some(_) | None => {}
    }

    if flags.contains(LinkFlags::LowerUp) && flags.contains(LinkFlags::Running) {
        CarrierState::Up
    } else {
        CarrierState::Down
    }
}

fn attribute(message: &LinkMessage) -> Option<u8> {
    message.attributes.iter().find_map(|attribute| {
        let LinkAttribute::Carrier(carrier) = *attribute else {
            return None;
        };

        Some(carrier)
    })
}
