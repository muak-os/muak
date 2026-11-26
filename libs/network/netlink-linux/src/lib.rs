//! Linux rtnetlink message builders and typed parsers that leave sequence and port numbers to the caller.

#![warn(missing_docs)]

mod carrier;
mod message;
mod parse;

use core::net::IpAddr;

use netlink_packet_core::{
    NLM_F_ACK, NLM_F_CREATE, NLM_F_DUMP, NLM_F_EXCL, NLM_F_REQUEST, NetlinkMessage,
};
use netlink_packet_route::address::{AddressAttribute, AddressMessage};
use netlink_packet_route::link::{
    BridgeStpState, InfoBridge, InfoData, InfoKind, LinkAttribute, LinkFlags, LinkInfo, LinkMessage,
};
use netlink_packet_route::{AddressFamily, RouteNetlinkMessage};
use netmodel::{Address, Link, LinkEvent, Name, Route};

/// A route netlink message with a finalized header.
pub type Message = NetlinkMessage<RouteNetlinkMessage>;

/// Builds a dump request for every link.
#[must_use = "the request must be sent or stored"]
pub fn get_links() -> Message {
    message::request(
        RouteNetlinkMessage::GetLink(LinkMessage::default()),
        NLM_F_REQUEST | NLM_F_DUMP,
    )
}

/// Builds a request for a single link identified by its kernel index.
#[must_use = "the request must be sent or stored"]
pub fn get_link(index: u32) -> Message {
    message::request(
        RouteNetlinkMessage::GetLink(message::link(index)),
        NLM_F_REQUEST,
    )
}

/// Builds a request that sets the `IFF_UP` flag on the link.
#[must_use = "the request must be sent or stored"]
pub fn set_link_up(index: u32) -> Message {
    let mut link = message::link(index);
    link.header.flags = LinkFlags::Up;
    link.header.change_mask = LinkFlags::Up;

    message::request(RouteNetlinkMessage::SetLink(link), message::modify_flags())
}

/// Builds a request that clears the `IFF_UP` flag on the link.
#[must_use = "the request must be sent or stored"]
pub fn set_link_down(index: u32) -> Message {
    let mut link = message::link(index);
    link.header.change_mask = LinkFlags::Up;

    message::request(RouteNetlinkMessage::SetLink(link), message::modify_flags())
}

/// Builds a request that attaches a slave link to a master link.
#[must_use = "the request must be sent or stored"]
pub fn set_master(slave: u32, master: u32) -> Message {
    let mut link = message::link(slave);
    link.attributes.push(LinkAttribute::Controller(master));

    message::request(RouteNetlinkMessage::SetLink(link), message::modify_flags())
}

/// Builds a request that detaches a link from its master.
#[must_use = "the request must be sent or stored"]
pub fn detach(index: u32) -> Message {
    let mut link = message::link(index);
    link.attributes.push(LinkAttribute::Controller(0));

    message::request(RouteNetlinkMessage::SetLink(link), message::modify_flags())
}

/// Builds a request that creates a Linux bridge.
#[must_use = "the request must be sent or stored"]
pub fn create_bridge(name: &Name, stp: bool) -> Message {
    let stp_state = if stp {
        BridgeStpState::KernelStp
    } else {
        BridgeStpState::Disabled
    };

    let mut link = LinkMessage::default();
    link.header.flags = LinkFlags::Up;
    link.header.change_mask = LinkFlags::Up;
    link.attributes
        .push(LinkAttribute::IfName(name.as_str().to_owned()));
    link.attributes.push(LinkAttribute::LinkInfo(vec![
        LinkInfo::Kind(InfoKind::Bridge),
        LinkInfo::Data(InfoData::Bridge(vec![InfoBridge::StpState(stp_state)])),
    ]));

    message::request(
        RouteNetlinkMessage::NewLink(link),
        NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL,
    )
}

/// Builds a dump request for every address.
#[must_use = "the request must be sent or stored"]
pub fn get_addresses() -> Message {
    message::request(
        RouteNetlinkMessage::GetAddress(AddressMessage::default()),
        NLM_F_REQUEST | NLM_F_DUMP,
    )
}

/// Builds a request that adds an address to the link at `index`.
#[must_use = "the request must be sent or stored"]
pub fn add_address(index: u32, address: IpAddr, prefix_len: u8) -> Message {
    let mut message = AddressMessage::default();
    message.header.index = index;
    message.header.prefix_len = prefix_len;

    match address {
        IpAddr::V4(address) => {
            message.header.family = AddressFamily::Inet;
            message::push_ipv4_address(&mut message.attributes, address, prefix_len);
        }
        IpAddr::V6(address) => {
            message.header.family = AddressFamily::Inet6;
            if address.is_multicast() {
                message
                    .attributes
                    .push(AddressAttribute::Multicast(address));
            } else {
                message
                    .attributes
                    .push(AddressAttribute::Address(address.into()));
                message
                    .attributes
                    .push(AddressAttribute::Local(address.into()));
            }
        }
    }

    message::request(
        RouteNetlinkMessage::NewAddress(message),
        message::modify_flags(),
    )
}

/// Builds a request that removes an address from the link at `index`.
#[must_use = "the request must be sent or stored"]
pub fn remove_address(index: u32, address: IpAddr, prefix_len: u8) -> Message {
    let mut message = AddressMessage::default();
    message.header.index = index;
    message.header.prefix_len = prefix_len;
    message.header.family = if address.is_ipv4() {
        AddressFamily::Inet
    } else {
        AddressFamily::Inet6
    };
    message.attributes.push(AddressAttribute::Address(address));
    message.attributes.push(AddressAttribute::Local(address));

    message::request(
        RouteNetlinkMessage::DelAddress(message),
        NLM_F_REQUEST | NLM_F_ACK,
    )
}

/// Builds a request that adds a default route through `gateway` on the link at `index`.
#[must_use = "the request must be sent or stored"]
pub fn add_route(index: u32, gateway: IpAddr) -> Message {
    message::request(
        RouteNetlinkMessage::NewRoute(message::route(index, gateway)),
        message::modify_flags(),
    )
}

/// Builds a request that removes the default route through `gateway` on the link at `index`.
#[must_use = "the request must be sent or stored"]
pub fn remove_route(index: u32, gateway: IpAddr) -> Message {
    message::request(
        RouteNetlinkMessage::DelRoute(message::route(index, gateway)),
        NLM_F_REQUEST | NLM_F_ACK,
    )
}

/// Parses a link snapshot from a link reply message.
#[must_use = "the parsed link must be stored or forwarded"]
pub fn parse_link(message: &RouteNetlinkMessage) -> Option<Link> {
    parse::link(message)
}

/// Parses an assigned address from an address reply message.
#[must_use = "the parsed address must be stored or forwarded"]
pub fn parse_address(message: &RouteNetlinkMessage) -> Option<Address> {
    parse::address(message)
}

/// Parses the link index and assigned address from an address reply message.
#[must_use = "the parsed index and address must be stored or forwarded"]
pub fn parse_address_index(message: &RouteNetlinkMessage) -> Option<(u32, Address)> {
    let (RouteNetlinkMessage::NewAddress(ref reply) | RouteNetlinkMessage::GetAddress(ref reply)) =
        *message
    else {
        return None;
    };

    Some((reply.header.index, parse::address(message)?))
}

/// Parses a route from a route reply message.
#[must_use = "the parsed route must be stored or forwarded"]
pub fn parse_route(message: &RouteNetlinkMessage) -> Option<Route> {
    parse::route(message)
}

/// Parses link lifecycle events from a route netlink notification.
///
/// `previous` is the caller's last known state for the link index. `None` marks an unknown link, so a `NewLink`
/// notification becomes [`LinkEvent::Added`]. A known link only emits [`LinkEvent::Up`] or [`LinkEvent::Down`] when its
/// carrier presence changed. `DelLink` always becomes [`LinkEvent::Deleted`]. [`LinkEvent::Error`] is not produced
/// here.
#[must_use = "the parsed event must be dispatched or dropped intentionally"]
pub fn parse_event(
    message: &RouteNetlinkMessage,
    previous: Option<netmodel::LinkState>,
) -> Option<LinkEvent> {
    parse::event(message, previous)
}
