//! Request construction tests for the netlink-linux message builders.

mod common;

use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use netlink_linux::{
    Message, add_address, add_route, create_bridge, detach, get_link, get_links, remove_address,
    remove_route, set_link_down, set_link_up, set_master,
};
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

const RTM_NEWLINK: u16 = 16;
const RTM_GETLINK: u16 = 18;
const RTM_SETLINK: u16 = 19;
const RTM_NEWADDR: u16 = 20;
const RTM_DELADDR: u16 = 21;
const RTM_NEWROUTE: u16 = 24;
const RTM_DELROUTE: u16 = 25;

#[cfg(test)]
mod tests {
    use super::*;

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
            0x20, 0x00, 0x00, 0x00, // length = 32
            0x12, 0x00, // message type = RTM_GETLINK
            0x01, 0x03, // flags = NLM_F_REQUEST | NLM_F_DUMP
            0x00, 0x00, 0x00, 0x00, // sequence number
            0x00, 0x00, 0x00, 0x00, // port number
            0x00, 0x00, 0x00, 0x00, // link header: family, reserved, layer type
            0x00, 0x00, 0x00, 0x00, // link index
            0x00, 0x00, 0x00, 0x00, // link flags
            0x00, 0x00, 0x00, 0x00, // link change mask
        ];

        // ACT
        let message = get_links();

        // ASSERT
        assert_eq!(message.header.message_type, RTM_GETLINK);
        assert_eq!(message.header.flags, NLM_F_REQUEST | NLM_F_DUMP);
        assert_eq!(common::emitted(&message), expected);
    }

    #[test]
    fn get_link_requests_index() {
        // ARRANGE
        let expected = vec![
            0x20, 0x00, 0x00, 0x00, // length = 32
            0x12, 0x00, // message type = RTM_GETLINK
            0x01, 0x00, // flags = NLM_F_REQUEST
            0x00, 0x00, 0x00, 0x00, // sequence number
            0x00, 0x00, 0x00, 0x00, // port number
            0x00, 0x00, 0x00, 0x00, // link header: family, reserved, layer type
            0x07, 0x00, 0x00, 0x00, // link index = 7
            0x00, 0x00, 0x00, 0x00, // link flags
            0x00, 0x00, 0x00, 0x00, // link change mask
        ];

        // ACT
        let message = get_link(7);

        // ASSERT
        assert_eq!(message.header.message_type, RTM_GETLINK);
        assert_eq!(message.header.flags, NLM_F_REQUEST);
        assert_eq!(common::emitted(&message), expected);
    }

    #[test]
    fn set_link_up_sets_flag_and_change_mask() {
        // ARRANGE
        let expected = vec![
            0x20, 0x00, 0x00, 0x00, // length = 32
            0x13, 0x00, // message type = RTM_SETLINK
            0x05, 0x06, // flags = NLM_F_REQUEST | NLM_F_ACK | NLM_F_EXCL | NLM_F_CREATE
            0x00, 0x00, 0x00, 0x00, // sequence number
            0x00, 0x00, 0x00, 0x00, // port number
            0x00, 0x00, 0x00, 0x00, // link header: family, reserved, layer type
            0x07, 0x00, 0x00, 0x00, // link index = 7
            0x01, 0x00, 0x00, 0x00, // link flags = IFF_UP
            0x01, 0x00, 0x00, 0x00, // link change mask = IFF_UP
        ];

        // ACT
        let message = set_link_up(7);

        // ASSERT
        assert_eq!(message.header.message_type, RTM_SETLINK);
        assert_eq!(
            message.header.flags,
            NLM_F_REQUEST | NLM_F_ACK | NLM_F_EXCL | NLM_F_CREATE
        );
        assert_eq!(common::emitted(&message), expected);
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
            0x28, 0x00, 0x00, 0x00, // length = 40
            0x15, 0x00, // message type = RTM_DELADDR
            0x05, 0x00, // flags = NLM_F_REQUEST | NLM_F_ACK
            0x00, 0x00, 0x00, 0x00, // sequence number
            0x00, 0x00, 0x00, 0x00, // port number
            0x02, 0x18, 0x00, 0x00, // family = inet, prefix = 24, flags, scope
            0x07, 0x00, 0x00, 0x00, // link index = 7
            0x08, 0x00, 0x01, 0x00, // IFA_ADDRESS
            0xc0, 0x00, 0x02, 0x0a, // 192.0.2.10
            0x08, 0x00, 0x02, 0x00, // IFA_LOCAL
            0xc0, 0x00, 0x02, 0x0a, // 192.0.2.10
        ];

        // ACT
        let message = remove_address(7, IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10)), 24);

        // ASSERT
        assert_eq!(common::emitted(&message), expected);
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
            0x44, 0x00, 0x00, 0x00, // length = 68
            0x10, 0x00, // message type = RTM_NEWLINK
            0x05, 0x06, // flags = NLM_F_REQUEST | NLM_F_ACK | NLM_F_EXCL | NLM_F_CREATE
            0x00, 0x00, 0x00, 0x00, // sequence number
            0x00, 0x00, 0x00, 0x00, // port number
            0x00, 0x00, 0x00, 0x00, // link header: family, reserved, layer type
            0x00, 0x00, 0x00, 0x00, // link index
            0x01, 0x00, 0x00, 0x00, // link flags = IFF_UP
            0x01, 0x00, 0x00, 0x00, // link change mask = IFF_UP
            0x08, 0x00, 0x03, 0x00, // IFLA_IFNAME
            0x62, 0x72, 0x30, 0x00, // "br0"
            0x1c, 0x00, 0x12, 0x00, // IFLA_LINKINFO
            0x0b, 0x00, 0x01, 0x00, // IFLA_INFO_KIND
            0x62, 0x72, 0x69, 0x64, 0x67, 0x65, 0x00, 0x00, // "bridge"
            0x0c, 0x00, 0x02, 0x00, // IFLA_INFO_DATA
            0x08, 0x00, 0x05, 0x00, // IFLA_BR_STP_STATE
            0x01, 0x00, 0x00, 0x00, // BR_KERNEL_STP
        ];

        // ACT
        let message = create_bridge(&name, true);

        // ASSERT
        assert_eq!(common::emitted(&message), expected);
    }

    #[test]
    fn add_ipv4_address_golden_request() {
        // ARRANGE
        let expected = vec![
            0x30, 0x00, 0x00, 0x00, // length = 48
            0x14, 0x00, // message type = RTM_NEWADDR
            0x05, 0x06, // flags = NLM_F_REQUEST | NLM_F_ACK | NLM_F_EXCL | NLM_F_CREATE
            0x00, 0x00, 0x00, 0x00, // sequence number
            0x00, 0x00, 0x00, 0x00, // port number
            0x02, 0x18, 0x00, 0x00, // family = inet, prefix = 24, flags, scope
            0x07, 0x00, 0x00, 0x00, // link index = 7
            0x08, 0x00, 0x01, 0x00, // IFA_ADDRESS
            0xc0, 0x00, 0x02, 0x0a, // 192.0.2.10
            0x08, 0x00, 0x02, 0x00, // IFA_LOCAL
            0xc0, 0x00, 0x02, 0x0a, // 192.0.2.10
            0x08, 0x00, 0x04, 0x00, // IFA_BROADCAST
            0xc0, 0x00, 0x02, 0xff, // 192.0.2.255
        ];

        // ACT
        let message = add_address(7, IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10)), 24);

        // ASSERT
        assert_eq!(common::emitted(&message), expected);
    }

    #[test]
    fn add_ipv4_route_golden_request() {
        // ARRANGE
        let expected = vec![
            0x2c, 0x00, 0x00, 0x00, // length = 44
            0x18, 0x00, // message type = RTM_NEWROUTE
            0x05, 0x06, // flags = NLM_F_REQUEST | NLM_F_ACK | NLM_F_EXCL | NLM_F_CREATE
            0x00, 0x00, 0x00, 0x00, // sequence number
            0x00, 0x00, 0x00, 0x00, // port number
            0x02, 0x00, 0x00, 0x00, // family = inet, prefixes, tos
            0xfe, 0x04, 0x00, 0x01, // table = main, protocol = static, scope, type
            0x00, 0x00, 0x00, 0x00, // route flags
            0x08, 0x00, 0x05, 0x00, // RTA_GATEWAY
            0xc0, 0x00, 0x02, 0x01, // 192.0.2.1
            0x08, 0x00, 0x04, 0x00, // RTA_OIF
            0x07, 0x00, 0x00, 0x00, // link index = 7
        ];

        // ACT
        let message = add_route(7, IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)));

        // ASSERT
        assert_eq!(common::emitted(&message), expected);
    }
}
