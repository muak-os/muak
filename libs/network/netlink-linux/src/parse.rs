//! Parsing of route netlink messages into domain values.

use core::net::IpAddr;

use netlink_packet_route::RouteNetlinkMessage;
use netlink_packet_route::address::AddressAttribute;
use netlink_packet_route::link::{LinkAttribute, LinkInfo, LinkMessage};
use netmodel::{Address, Link, LinkEvent, LinkState, Mac, Name};

use crate::carrier;

pub(crate) fn link(message: &RouteNetlinkMessage) -> Option<Link> {
    let (RouteNetlinkMessage::NewLink(ref message) | RouteNetlinkMessage::GetLink(ref message)) =
        *message
    else {
        return None;
    };

    let (admin, carrier) = carrier::parts(message);

    Some(Link {
        name: link_name(message)?,
        index: message.header.index,
        mac: link_mac(message),
        admin,
        carrier,
        is_virtual: is_virtual_link(message),
        master: link_master(message),
    })
}

pub(crate) fn address(message: &RouteNetlinkMessage) -> Option<Address> {
    let (RouteNetlinkMessage::NewAddress(ref message)
    | RouteNetlinkMessage::GetAddress(ref message)) = *message
    else {
        return None;
    };

    let address = message.attributes.iter().find_map(address_attribute)?;

    Some(Address {
        address,
        prefix_len: message.header.prefix_len,
    })
}

pub(crate) fn event(
    message: &RouteNetlinkMessage,
    previous: Option<LinkState>,
) -> Option<LinkEvent> {
    if let RouteNetlinkMessage::DelLink(ref message) = *message {
        return Some(LinkEvent::Deleted {
            name: link_name(message)?,
            index: message.header.index,
        });
    }

    let RouteNetlinkMessage::NewLink(ref message) = *message else {
        return None;
    };
    let name = link_name(message)?;
    let index = message.header.index;
    let Some(previous) = previous else {
        return Some(LinkEvent::Added {
            name,
            index,
            mac: Mac::new(link_mac_array(message)?),
        });
    };

    carrier_event(previous, message, name, index)
}

fn carrier_event(
    previous: LinkState,
    message: &LinkMessage,
    name: Name,
    index: u32,
) -> Option<LinkEvent> {
    let current = carrier::state(message);

    match (previous.has_carrier(), current.has_carrier()) {
        (false, true) => Some(LinkEvent::Up { name, index }),
        (true, false) => Some(LinkEvent::Down { name, index }),
        (false, false) | (true, true) => None,
    }
}

fn link_name(message: &LinkMessage) -> Option<Name> {
    message.attributes.iter().find_map(|attribute| {
        let LinkAttribute::IfName(ref name) = *attribute else {
            return None;
        };
        Name::new(name.clone()).ok()
    })
}

fn link_mac_array(message: &LinkMessage) -> Option<[u8; 6]> {
    message.attributes.iter().find_map(|attribute| {
        let LinkAttribute::Address(ref bytes) = *attribute else {
            return None;
        };
        <[u8; 6]>::try_from(bytes.as_slice()).ok()
    })
}

fn link_mac(message: &LinkMessage) -> Mac {
    link_mac_array(message).map_or_else(Mac::default, Mac::new)
}

fn is_virtual_link(message: &LinkMessage) -> bool {
    message.attributes.iter().any(|attribute| {
        let LinkAttribute::LinkInfo(ref infos) = *attribute else {
            return false;
        };
        infos.iter().any(|info| matches!(info, LinkInfo::Kind(_)))
    })
}

fn link_master(message: &LinkMessage) -> Option<u32> {
    message.attributes.iter().find_map(|attribute| {
        let LinkAttribute::Controller(ref master) = *attribute else {
            return None;
        };
        (*master != 0).then_some(*master)
    })
}

const fn address_attribute(attribute: &AddressAttribute) -> Option<IpAddr> {
    let AddressAttribute::Address(ref address) = *attribute else {
        return None;
    };

    Some(*address)
}
