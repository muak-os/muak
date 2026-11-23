//! Netlink message construction helpers shared by the public builders.

use core::net::{IpAddr, Ipv4Addr};

use netlink_packet_core::{NLM_F_ACK, NLM_F_CREATE, NLM_F_EXCL, NLM_F_REQUEST};
use netlink_packet_route::address::AddressAttribute;
use netlink_packet_route::link::{LinkHeader, LinkMessage};
use netlink_packet_route::route::{
    RouteAddress, RouteAttribute, RouteHeader, RouteMessage, RouteProtocol, RouteScope, RouteType,
};
use netlink_packet_route::{AddressFamily, RouteNetlinkMessage};

use crate::Message;

pub(crate) fn request(payload: RouteNetlinkMessage, flags: u16) -> Message {
    let mut message = Message::from(payload);
    message.header.flags = flags;
    message.finalize();
    message
}

pub(crate) const fn modify_flags() -> u16 {
    NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL
}

pub(crate) fn push_ipv4_address(
    attributes: &mut Vec<AddressAttribute>,
    address: Ipv4Addr,
    prefix_len: u8,
) {
    if address.is_multicast() {
        return;
    }

    attributes.push(AddressAttribute::Address(address.into()));
    attributes.push(AddressAttribute::Local(address.into()));
    if prefix_len < 31 {
        let broadcast = Ipv4Addr::from(u32::from(address) | (0xffff_ffff_u32 >> prefix_len));
        attributes.push(AddressAttribute::Broadcast(broadcast));
    }
}

pub(crate) fn link(index: u32) -> LinkMessage {
    let mut message = LinkMessage::default();
    message.header = LinkHeader {
        index,
        ..LinkHeader::default()
    };
    message
}

pub(crate) fn route(index: u32, gateway: IpAddr) -> RouteMessage {
    let mut message = RouteMessage::default();
    message.header.table = RouteHeader::RT_TABLE_MAIN;
    message.header.protocol = RouteProtocol::Static;
    message.header.scope = RouteScope::Universe;
    message.header.kind = RouteType::Unicast;

    match gateway {
        IpAddr::V4(gateway) => {
            message.header.address_family = AddressFamily::Inet;
            message
                .attributes
                .push(RouteAttribute::Gateway(RouteAddress::Inet(gateway)));
        }
        IpAddr::V6(gateway) => {
            message.header.address_family = AddressFamily::Inet6;
            message
                .attributes
                .push(RouteAttribute::Gateway(RouteAddress::Inet6(gateway)));
        }
    }
    message.attributes.push(RouteAttribute::Oif(index));

    message
}
