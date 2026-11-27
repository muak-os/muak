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
use netmodel::{Address, Link, LinkEvent, Name};

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

/// Parses link lifecycle events from a route netlink notification.
#[must_use = "the parsed event must be dispatched or dropped intentionally"]
pub fn parse_event(
    message: &RouteNetlinkMessage,
    previous: Option<netmodel::LinkState>,
) -> Option<LinkEvent> {
    parse::event(message, previous)
}

#[cfg(test)]
mod tests {
    use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use netlink_packet_core::{
        NLM_F_ACK, NLM_F_CREATE, NLM_F_DUMP, NLM_F_EXCL, NLM_F_REQUEST, NetlinkPayload,
    };
    use netlink_packet_route::address::AddressAttribute;
    use netlink_packet_route::link::{
        BridgeStpState, InfoBridge, InfoData, InfoKind, LinkAttribute, LinkFlags, LinkInfo,
    };
    use netlink_packet_route::route::{RouteAddress, RouteAttribute, RouteHeader, RouteProtocol};
    use netlink_packet_route::{AddressFamily, RouteNetlinkMessage};
    use netmodel::Name;

    use super::*;

    const RTM_NEWLINK: u16 = 16;
    const RTM_GETLINK: u16 = 18;
    const RTM_SETLINK: u16 = 19;
    const RTM_NEWADDR: u16 = 20;
    const RTM_DELADDR: u16 = 21;
    const RTM_GETADDR: u16 = 22;
    const RTM_NEWROUTE: u16 = 24;
    const RTM_DELROUTE: u16 = 25;

    fn emitted(message: &Message) -> Vec<u8> {
        let mut buffer = vec![0_u8; message.buffer_len()];
        message.serialize(&mut buffer);
        buffer
    }

    fn inner(message: &Message) -> &RouteNetlinkMessage {
        let NetlinkPayload::InnerMessage(ref payload) = message.payload else {
            panic!("expected an inner route netlink message");
        };

        payload
    }

    #[test]
    fn get_links_golden_request() {
        // ARRANGE
        let expected = vec![
            0x20, 0x00, 0x00, 0x00, 0x12, 0x00, 0x01, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00,
        ];

        // ACT
        let message = get_links();

        // ASSERT
        assert_eq!(message.header.message_type, RTM_GETLINK);
        assert_eq!(message.header.flags, NLM_F_REQUEST | NLM_F_DUMP);
        assert_eq!(emitted(&message), expected);
    }

    #[test]
    fn get_link_requests_index() {
        // ARRANGE
        let expected = vec![
            0x20, 0x00, 0x00, 0x00, 0x12, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00,
        ];

        // ACT
        let message = get_link(7);

        // ASSERT
        assert_eq!(message.header.message_type, RTM_GETLINK);
        assert_eq!(message.header.flags, NLM_F_REQUEST);
        assert_eq!(emitted(&message), expected);
    }

    #[test]
    fn set_link_up_sets_flag_and_change_mask() {
        // ARRANGE
        let expected = vec![
            0x20, 0x00, 0x00, 0x00, 0x13, 0x00, 0x05, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x01, 0x00, 0x00, 0x00,
        ];

        // ACT
        let message = set_link_up(7);

        // ASSERT
        assert_eq!(message.header.message_type, RTM_SETLINK);
        assert_eq!(
            message.header.flags,
            NLM_F_REQUEST | NLM_F_ACK | NLM_F_EXCL | NLM_F_CREATE
        );
        assert_eq!(emitted(&message), expected);
    }

    #[test]
    fn set_link_down_clears_flag_through_change_mask() {
        // ARRANGE
        let message = set_link_down(7);
        let RouteNetlinkMessage::SetLink(ref link) = *inner(&message) else {
            panic!("expected SetLink");
        };

        // ACT / ASSERT
        assert_eq!(link.header.index, 7);
        assert!(!link.header.flags.contains(LinkFlags::Up));
        assert_eq!(link.header.change_mask, LinkFlags::Up);
    }

    #[test]
    fn set_master_attaches_controller() {
        // ARRANGE
        let message = set_master(7, 3);

        // ACT
        let RouteNetlinkMessage::SetLink(ref link) = *inner(&message) else {
            panic!("expected SetLink");
        };

        // ASSERT
        assert_eq!(link.header.index, 7);
        assert_eq!(link.attributes, vec![LinkAttribute::Controller(3)]);
    }

    #[test]
    fn detach_uses_zero_controller() {
        // ARRANGE
        let message = detach(7);

        // ACT
        let RouteNetlinkMessage::SetLink(ref link) = *inner(&message) else {
            panic!("expected SetLink");
        };

        // ASSERT
        assert_eq!(link.attributes, vec![LinkAttribute::Controller(0)]);
    }

    #[test]
    fn create_bridge_builds_newlink_with_kind_and_stp() {
        // ARRANGE
        let name = Name::new("br0").expect("valid name");

        // ACT
        let message = create_bridge(&name, true);

        // ASSERT
        assert_eq!(message.header.message_type, RTM_NEWLINK);
        assert_eq!(
            message.header.flags,
            NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL
        );
        let RouteNetlinkMessage::NewLink(ref link) = *inner(&message) else {
            panic!("expected NewLink");
        };
        assert_eq!(link.header.flags, LinkFlags::Up);
        assert_eq!(link.header.change_mask, LinkFlags::Up);
        assert_eq!(
            link.attributes,
            vec![
                LinkAttribute::IfName("br0".to_owned()),
                LinkAttribute::LinkInfo(vec![
                    LinkInfo::Kind(InfoKind::Bridge),
                    LinkInfo::Data(InfoData::Bridge(vec![InfoBridge::StpState(
                        BridgeStpState::KernelStp,
                    )])),
                ]),
            ]
        );
    }

    #[test]
    fn get_addresses_requests_a_dump() {
        // ARRANGE
        let dump = NLM_F_REQUEST | NLM_F_DUMP;

        // ACT
        let message = get_addresses();

        // ASSERT
        assert_eq!(message.header.message_type, RTM_GETADDR);
        assert_eq!(message.header.flags, dump);
    }

    #[test]
    fn add_ipv4_address_builds_address_local_and_broadcast() {
        // ARRANGE
        let address = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10));

        // ACT
        let message = add_address(7, address, 24);

        // ASSERT
        assert_eq!(message.header.message_type, RTM_NEWADDR);
        assert_eq!(
            message.header.flags,
            NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL
        );
        let RouteNetlinkMessage::NewAddress(ref address_message) = *inner(&message) else {
            panic!("expected NewAddress");
        };
        assert_eq!(address_message.header.index, 7);
        assert_eq!(address_message.header.family, AddressFamily::Inet);
        assert_eq!(address_message.header.prefix_len, 24);
        assert_eq!(
            address_message.attributes,
            vec![
                AddressAttribute::Address(address),
                AddressAttribute::Local(address),
                AddressAttribute::Broadcast(Ipv4Addr::new(192, 0, 2, 255)),
            ]
        );
    }

    #[test]
    fn add_ipv6_address_skips_broadcast() {
        // ARRANGE
        let address = IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1));

        // ACT
        let message = add_address(7, address, 64);

        // ASSERT
        let RouteNetlinkMessage::NewAddress(ref address_message) = *inner(&message) else {
            panic!("expected NewAddress");
        };
        assert_eq!(address_message.header.family, AddressFamily::Inet6);
        assert_eq!(
            address_message.attributes,
            vec![
                AddressAttribute::Address(address),
                AddressAttribute::Local(address),
            ]
        );
    }

    #[test]
    fn add_ipv4_multicast_address_skips_link_attributes() {
        // ARRANGE
        let address = IpAddr::V4(Ipv4Addr::new(224, 0, 0, 1));

        // ACT
        let message = add_address(7, address, 4);

        // ASSERT
        let RouteNetlinkMessage::NewAddress(ref address_message) = *inner(&message) else {
            panic!("expected NewAddress");
        };
        assert_eq!(address_message.header.family, AddressFamily::Inet);
        assert_eq!(address_message.header.prefix_len, 4);
        assert_eq!(address_message.attributes, []);
    }

    #[test]
    fn remove_address_requests_delete_by_index_and_address() {
        // ARRANGE
        let address = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10));

        // ACT
        let message = remove_address(7, address, 24);

        // ASSERT
        assert_eq!(message.header.message_type, RTM_DELADDR);
        assert_eq!(message.header.flags, NLM_F_REQUEST | NLM_F_ACK);
        let RouteNetlinkMessage::DelAddress(ref address_message) = *inner(&message) else {
            panic!("expected DelAddress");
        };
        assert_eq!(address_message.header.index, 7);
        assert_eq!(address_message.header.family, AddressFamily::Inet);
        assert_eq!(address_message.header.prefix_len, 24);
        assert_eq!(
            address_message.attributes,
            vec![
                AddressAttribute::Address(address),
                AddressAttribute::Local(address),
            ]
        );
    }

    #[test]
    fn remove_ipv4_address_golden_request() {
        // ARRANGE
        let expected = vec![
            0x28, 0x00, 0x00, 0x00, 0x15, 0x00, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x02, 0x18, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x08, 0x00, 0x01, 0x00,
            0xc0, 0x00, 0x02, 0x0a, 0x08, 0x00, 0x02, 0x00, 0xc0, 0x00, 0x02, 0x0a,
        ];

        // ACT
        let message = remove_address(7, IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10)), 24);

        // ASSERT
        assert_eq!(emitted(&message), expected);
    }

    #[test]
    fn add_ipv4_route_includes_gateway_and_output_interface() {
        // ARRANGE
        let gateway = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));

        // ACT
        let message = add_route(7, gateway);

        // ASSERT
        assert_eq!(message.header.message_type, RTM_NEWROUTE);
        assert_eq!(
            message.header.flags,
            NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL
        );
        let RouteNetlinkMessage::NewRoute(ref route) = *inner(&message) else {
            panic!("expected NewRoute");
        };
        assert_eq!(route.header.address_family, AddressFamily::Inet);
        assert_eq!(route.header.table, RouteHeader::RT_TABLE_MAIN);
        assert_eq!(route.header.protocol, RouteProtocol::Static);
        assert_eq!(
            route.attributes,
            vec![
                RouteAttribute::Gateway(RouteAddress::Inet(Ipv4Addr::new(192, 0, 2, 1))),
                RouteAttribute::Oif(7),
            ]
        );
    }

    #[test]
    fn remove_ipv6_route_requests_delete() {
        // ARRANGE
        let gateway = IpAddr::V6(Ipv6Addr::LOCALHOST);

        // ACT
        let message = remove_route(7, gateway);

        // ASSERT
        assert_eq!(message.header.message_type, RTM_DELROUTE);
        assert_eq!(message.header.flags, NLM_F_REQUEST | NLM_F_ACK);
        let RouteNetlinkMessage::DelRoute(ref route) = *inner(&message) else {
            panic!("expected DelRoute");
        };
        assert_eq!(route.header.address_family, AddressFamily::Inet6);
        assert_eq!(
            route.attributes,
            vec![
                RouteAttribute::Gateway(RouteAddress::Inet6(Ipv6Addr::LOCALHOST)),
                RouteAttribute::Oif(7),
            ]
        );
    }

    #[test]
    fn create_bridge_golden_request() {
        // ARRANGE
        let name = Name::new("br0").expect("valid name");
        let expected = vec![
            0x44, 0x00, 0x00, 0x00, 0x10, 0x00, 0x05, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x01, 0x00, 0x00, 0x00, 0x08, 0x00, 0x03, 0x00, 0x62, 0x72, 0x30, 0x00, 0x1c, 0x00,
            0x12, 0x00, 0x0b, 0x00, 0x01, 0x00, 0x62, 0x72, 0x69, 0x64, 0x67, 0x65, 0x00, 0x00,
            0x0c, 0x00, 0x02, 0x00, 0x08, 0x00, 0x05, 0x00, 0x01, 0x00, 0x00, 0x00,
        ];

        // ACT
        let message = create_bridge(&name, true);

        // ASSERT
        assert_eq!(emitted(&message), expected);
    }

    #[test]
    fn add_ipv4_address_golden_request() {
        // ARRANGE
        let expected = vec![
            0x30, 0x00, 0x00, 0x00, 0x14, 0x00, 0x05, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x02, 0x18, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x08, 0x00, 0x01, 0x00,
            0xc0, 0x00, 0x02, 0x0a, 0x08, 0x00, 0x02, 0x00, 0xc0, 0x00, 0x02, 0x0a, 0x08, 0x00,
            0x04, 0x00, 0xc0, 0x00, 0x02, 0xff,
        ];

        // ACT
        let message = add_address(7, IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10)), 24);

        // ASSERT
        assert_eq!(emitted(&message), expected);
    }

    #[test]
    fn add_ipv4_route_golden_request() {
        // ARRANGE
        let expected = vec![
            0x2c, 0x00, 0x00, 0x00, 0x18, 0x00, 0x05, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0xfe, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
            0x08, 0x00, 0x05, 0x00, 0xc0, 0x00, 0x02, 0x01, 0x08, 0x00, 0x04, 0x00, 0x07, 0x00,
            0x00, 0x00,
        ];

        // ACT
        let message = add_route(7, IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)));

        // ASSERT
        assert_eq!(emitted(&message), expected);
    }
}
