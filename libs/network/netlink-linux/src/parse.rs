//! Parsing of route netlink messages into domain values.

use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use netlink_packet_route::address::AddressAttribute;
use netlink_packet_route::link::{LinkAttribute, LinkFlags, LinkInfo, LinkMessage};
use netlink_packet_route::route::{RouteAddress, RouteAttribute};
use netlink_packet_route::{AddressFamily, RouteNetlinkMessage};
use netmodel::{Address, Link, LinkEvent, LinkState, Mac, Name, Route};

pub(crate) fn link(message: &RouteNetlinkMessage) -> Option<Link> {
    let (RouteNetlinkMessage::NewLink(ref message) | RouteNetlinkMessage::GetLink(ref message)) =
        *message
    else {
        return None;
    };

    Some(Link {
        name: link_name(message)?,
        index: message.header.index,
        mac: link_mac(message),
        state: link_state(message.header.flags),
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

pub(crate) fn route(message: &RouteNetlinkMessage) -> Option<Route> {
    let (RouteNetlinkMessage::NewRoute(ref message) | RouteNetlinkMessage::GetRoute(ref message)) =
        *message
    else {
        return None;
    };

    let destination = message
        .attributes
        .iter()
        .find_map(route_address)
        .or_else(|| unspecified(message.header.address_family))?;
    let gateway = message.attributes.iter().find_map(route_gateway);

    Some(Route {
        destination,
        prefix_len: message.header.destination_prefix_length,
        gateway,
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

    carrier_event(previous, message.header.flags, name, index)
}

fn carrier_event(
    previous: LinkState,
    flags: LinkFlags,
    name: Name,
    index: u32,
) -> Option<LinkEvent> {
    let was_present = matches!(previous, LinkState::Up);
    let is_present = flags.contains(LinkFlags::LowerUp);

    match (was_present, is_present) {
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

const fn link_state(flags: LinkFlags) -> LinkState {
    match (
        flags.contains(LinkFlags::Up),
        flags.contains(LinkFlags::LowerUp),
    ) {
        (false, false | true) => LinkState::Down,
        (true, true) => LinkState::Up,
        (true, false) => LinkState::NoCarrier,
    }
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

const fn route_address(attribute: &RouteAttribute) -> Option<IpAddr> {
    let RouteAttribute::Destination(ref address) = *attribute else {
        return None;
    };

    route_addr(address)
}

const fn route_gateway(attribute: &RouteAttribute) -> Option<IpAddr> {
    let RouteAttribute::Gateway(ref address) = *attribute else {
        return None;
    };

    route_addr(address)
}

const fn route_addr(address: &RouteAddress) -> Option<IpAddr> {
    if let RouteAddress::Inet(ref address) = *address {
        return Some(IpAddr::V4(*address));
    }
    if let RouteAddress::Inet6(ref address) = *address {
        return Some(IpAddr::V6(*address));
    }

    None
}

const fn unspecified(family: AddressFamily) -> Option<IpAddr> {
    if matches!(family, AddressFamily::Inet) {
        return Some(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    }
    if matches!(family, AddressFamily::Inet6) {
        return Some(IpAddr::V6(Ipv6Addr::UNSPECIFIED));
    }

    None
}
