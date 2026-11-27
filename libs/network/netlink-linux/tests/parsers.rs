//! Reply and notification parsing tests for netlink-linux.

use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use netlink_linux::{parse_address, parse_address_index, parse_event, parse_link};
use netlink_packet_route::address::{AddressAttribute, AddressMessage};
use netlink_packet_route::link::{
    InfoKind, LinkAttribute, LinkFlags, LinkInfo, LinkMessage, State as OperState,
};
use netlink_packet_route::route::RouteMessage;
use netlink_packet_route::{AddressFamily, RouteNetlinkMessage};
use netmodel::{Address, AdminState, CarrierState, Link, LinkEvent, LinkState, Mac};

fn link(index: u32, name: &str, flags: LinkFlags, change_mask: LinkFlags) -> LinkMessage {
    let mut message = LinkMessage::default();
    message.header.index = index;
    message.header.flags = flags;
    message.header.change_mask = change_mask;
    message
        .attributes
        .push(LinkAttribute::IfName(name.to_owned()));

    message
}

fn link_with_state(
    index: u32,
    name: &str,
    flags: LinkFlags,
    operstate: Option<OperState>,
    carrier: Option<u8>,
) -> RouteNetlinkMessage {
    let mut message = link(index, name, flags, LinkFlags::empty());
    if let Some(operstate) = operstate {
        message.attributes.push(LinkAttribute::OperState(operstate));
    }
    if let Some(carrier) = carrier {
        message.attributes.push(LinkAttribute::Carrier(carrier));
    }

    RouteNetlinkMessage::NewLink(message)
}

fn address(index: u32, address: IpAddr, prefix_len: u8) -> AddressMessage {
    let mut message = AddressMessage::default();
    message.header.index = index;
    message.header.prefix_len = prefix_len;
    message.header.family = if address.is_ipv4() {
        AddressFamily::Inet
    } else {
        AddressFamily::Inet6
    };
    message.attributes.push(AddressAttribute::Address(address));

    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_link_reads_domain_snapshot() {
        // ARRANGE
        let mut message = link(
            7,
            "eth0",
            LinkFlags::Up | LinkFlags::LowerUp,
            LinkFlags::empty(),
        );
        message
            .attributes
            .push(LinkAttribute::Address(vec![1, 2, 3, 4, 5, 6]));
        message.attributes.push(LinkAttribute::Controller(3));
        message
            .attributes
            .push(LinkAttribute::LinkInfo(vec![LinkInfo::Kind(
                InfoKind::Veth,
            )]));
        message
            .attributes
            .push(LinkAttribute::OperState(OperState::Up));
        let message = RouteNetlinkMessage::NewLink(message);

        // ACT
        let parsed = parse_link(&message);

        // ASSERT
        assert_eq!(
            parsed,
            Some(Link {
                name: "eth0".parse().expect("valid name"),
                index: 7,
                mac: Mac::new([1, 2, 3, 4, 5, 6]),
                admin: AdminState::Up,
                carrier: CarrierState::Up,
                is_virtual: true,
                master: Some(3),
            })
        );
    }

    #[test]
    fn parse_link_maps_admin_and_carrier_flags() {
        // ARRANGE
        let admin_down =
            RouteNetlinkMessage::NewLink(link(1, "eth0", LinkFlags::empty(), LinkFlags::empty()));
        let no_carrier =
            RouteNetlinkMessage::NewLink(link(2, "eth1", LinkFlags::Up, LinkFlags::empty()));

        // ACT
        let down = parse_link(&admin_down);
        let carrier = parse_link(&no_carrier);

        // ASSERT
        assert_eq!(down.map(|link| link.state()), Some(LinkState::Down));
        assert_eq!(carrier.map(|link| link.state()), Some(LinkState::NoCarrier));
    }

    #[test]
    fn parse_link_admin_down_wins_over_operstate_up() {
        // ARRANGE
        let message = link_with_state(1, "eth0", LinkFlags::empty(), Some(OperState::Up), None);

        // ACT
        let parsed = parse_link(&message);

        // ASSERT
        assert_eq!(parsed.map(|link| link.state()), Some(LinkState::Down));
    }

    #[test]
    fn parse_link_operstate_up_wins_over_missing_lower_up() {
        // ARRANGE
        let message = link_with_state(1, "eth0", LinkFlags::Up, Some(OperState::Up), None);

        // ACT
        let parsed = parse_link(&message);

        // ASSERT
        assert_eq!(parsed.map(|link| link.state()), Some(LinkState::Up));
    }

    #[test]
    fn parse_link_operstate_down_wins_over_lower_up_flags() {
        // ARRANGE
        let message = link_with_state(
            1,
            "eth0",
            LinkFlags::Up | LinkFlags::LowerUp | LinkFlags::Running,
            Some(OperState::Down),
            Some(1),
        );

        // ACT
        let parsed = parse_link(&message);

        // ASSERT
        assert_eq!(parsed.map(|link| link.state()), Some(LinkState::NoCarrier));
    }

    #[test]
    fn parse_link_operstate_dormant_becomes_dormant() {
        // ARRANGE
        let message = link_with_state(
            1,
            "eth0",
            LinkFlags::Up | LinkFlags::Dormant,
            Some(OperState::Dormant),
            Some(1),
        );

        // ACT
        let parsed = parse_link(&message);

        // ASSERT
        assert_eq!(parsed.map(|link| link.state()), Some(LinkState::Dormant));
    }

    #[test]
    fn parse_link_unknown_operstate_falls_back_to_lower_up_and_running() {
        // ARRANGE
        let message = link_with_state(
            1,
            "eth0",
            LinkFlags::Up | LinkFlags::LowerUp | LinkFlags::Running,
            Some(OperState::Unknown),
            None,
        );

        // ACT
        let parsed = parse_link(&message);

        // ASSERT
        assert_eq!(parsed.map(|link| link.state()), Some(LinkState::Up));
    }

    #[test]
    fn parse_link_missing_operstate_falls_back_to_ifla_carrier() {
        // ARRANGE
        let up = link_with_state(1, "eth0", LinkFlags::Up, None, Some(1));
        let down = link_with_state(2, "eth1", LinkFlags::Up, None, Some(0));

        // ACT
        let parsed_up = parse_link(&up);
        let parsed_down = parse_link(&down);

        // ASSERT
        assert_eq!(parsed_up.map(|link| link.state()), Some(LinkState::Up));
        assert_eq!(
            parsed_down.map(|link| link.state()),
            Some(LinkState::NoCarrier)
        );
    }

    #[test]
    fn parse_link_unknown_operstate_falls_back_to_dormant_flag() {
        // ARRANGE
        let message = link_with_state(
            1,
            "eth0",
            LinkFlags::Up | LinkFlags::Dormant,
            Some(OperState::Unknown),
            None,
        );

        // ACT
        let parsed = parse_link(&message);

        // ASSERT
        assert_eq!(parsed.map(|link| link.state()), Some(LinkState::Dormant));
    }

    #[test]
    fn parse_link_missing_state_reports_no_carrier() {
        // ARRANGE
        let message = link_with_state(1, "eth0", LinkFlags::Up, None, None);

        // ACT
        let parsed = parse_link(&message);

        // ASSERT
        assert_eq!(parsed.map(|link| link.state()), Some(LinkState::NoCarrier));
    }

    #[test]
    fn parse_link_rejects_other_messages_and_missing_name() {
        // ARRANGE
        let other = RouteNetlinkMessage::GetRoute(RouteMessage::default());
        let unnamed = RouteNetlinkMessage::NewLink(LinkMessage::default());

        // ACT / ASSERT
        assert!(parse_link(&other).is_none());
        assert!(parse_link(&unnamed).is_none());
    }

    #[test]
    fn parse_address_reads_address_and_prefix() {
        // ARRANGE
        let message = RouteNetlinkMessage::NewAddress(address(
            7,
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10)),
            24,
        ));

        // ACT
        let parsed = parse_address(&message);

        // ASSERT
        assert_eq!(
            parsed,
            Some(Address {
                address: IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10)),
                prefix_len: 24,
            })
        );
    }

    #[test]
    fn parse_address_index_reads_index_and_address() {
        // ARRANGE
        let message =
            RouteNetlinkMessage::GetAddress(address(5, IpAddr::V6(Ipv6Addr::LOCALHOST), 128));
        let expected = (
            5,
            Address {
                address: IpAddr::V6(Ipv6Addr::LOCALHOST),
                prefix_len: 128,
            },
        );

        // ACT
        let parsed = parse_address_index(&message);

        // ASSERT
        assert_eq!(parsed, Some(expected));
    }

    #[test]
    fn parse_address_index_reads_new_address_replies() {
        // ARRANGE
        let message = RouteNetlinkMessage::NewAddress(address(
            9,
            IpAddr::V4(Ipv4Addr::new(198, 51, 100, 2)),
            24,
        ));

        // ACT
        let parsed = parse_address_index(&message);

        // ASSERT
        assert_eq!(
            parsed,
            Some((
                9,
                Address {
                    address: IpAddr::V4(Ipv4Addr::new(198, 51, 100, 2)),
                    prefix_len: 24,
                },
            ))
        );
    }

    #[test]
    fn parse_address_index_rejects_non_address_messages() {
        // ARRANGE
        let message = RouteNetlinkMessage::NewLink(LinkMessage::default());

        // ACT
        let parsed = parse_address_index(&message);

        // ASSERT
        assert_eq!(parsed, None);
    }

    #[test]
    fn parse_event_emits_added_for_unknown_link() {
        // ARRANGE
        let mut message = link(7, "eth0", LinkFlags::empty(), LinkFlags::all());
        message
            .attributes
            .push(LinkAttribute::Address(vec![1, 2, 3, 4, 5, 6]));
        let message = RouteNetlinkMessage::NewLink(message);

        // ACT
        let parsed = parse_event(&message, None);

        // ASSERT
        assert_eq!(
            parsed,
            Some(LinkEvent::Added {
                name: "eth0".parse().expect("valid name"),
                index: 7,
                mac: Mac::new([1, 2, 3, 4, 5, 6]),
            })
        );
    }

    #[test]
    fn parse_event_emits_deleted_for_del_link() {
        // ARRANGE
        let message =
            RouteNetlinkMessage::DelLink(link(7, "eth0", LinkFlags::empty(), LinkFlags::empty()));

        // ACT
        let parsed = parse_event(&message, Some(LinkState::Up));

        // ASSERT
        assert_eq!(
            parsed,
            Some(LinkEvent::Deleted {
                name: "eth0".parse().expect("valid name"),
                index: 7,
            })
        );
    }

    #[test]
    fn parse_event_emits_up_on_carrier_gain() {
        // ARRANGE
        let message = RouteNetlinkMessage::NewLink(link(
            7,
            "eth0",
            LinkFlags::Up | LinkFlags::LowerUp | LinkFlags::Running,
            LinkFlags::LowerUp,
        ));

        // ACT
        let parsed = parse_event(&message, Some(LinkState::NoCarrier));

        // ASSERT
        assert_eq!(
            parsed,
            Some(LinkEvent::Up {
                name: "eth0".parse().expect("valid name"),
                index: 7,
            })
        );
    }

    #[test]
    fn parse_event_emits_down_on_carrier_loss() {
        // ARRANGE
        let message =
            RouteNetlinkMessage::NewLink(link(7, "eth0", LinkFlags::Up, LinkFlags::empty()));

        // ACT
        let parsed = parse_event(&message, Some(LinkState::Up));

        // ASSERT
        assert_eq!(
            parsed,
            Some(LinkEvent::Down {
                name: "eth0".parse().expect("valid name"),
                index: 7,
            })
        );
    }

    #[test]
    fn parse_event_ignores_unchanged_carrier_for_known_link() {
        // ARRANGE
        let present = RouteNetlinkMessage::NewLink(link(
            7,
            "eth0",
            LinkFlags::Up | LinkFlags::LowerUp | LinkFlags::Running,
            LinkFlags::empty(),
        ));
        let absent =
            RouteNetlinkMessage::NewLink(link(7, "eth0", LinkFlags::Up, LinkFlags::empty()));

        // ACT
        let unchanged_present = parse_event(&present, Some(LinkState::Up));
        let unchanged_absent = parse_event(&absent, Some(LinkState::Down));

        // ASSERT
        assert!(unchanged_present.is_none());
        assert!(unchanged_absent.is_none());
    }

    #[test]
    fn parse_event_emits_up_across_a_dormant_transition() {
        // ARRANGE
        let message = link_with_state(7, "eth0", LinkFlags::Up, Some(OperState::Up), None);

        // ACT
        let parsed = parse_event(&message, Some(LinkState::Dormant));

        // ASSERT
        assert_eq!(
            parsed,
            Some(LinkEvent::Up {
                name: "eth0".parse().expect("valid name"),
                index: 7,
            })
        );
    }

    #[test]
    fn parse_event_emits_down_across_an_admin_transition() {
        // ARRANGE
        let message = link_with_state(7, "eth0", LinkFlags::empty(), Some(OperState::Up), None);

        // ACT
        let parsed = parse_event(&message, Some(LinkState::Up));

        // ASSERT
        assert_eq!(
            parsed,
            Some(LinkEvent::Down {
                name: "eth0".parse().expect("valid name"),
                index: 7,
            })
        );
    }

    #[test]
    fn parse_event_ignores_unchanged_dormant_carrier() {
        // ARRANGE
        let message = link_with_state(
            7,
            "eth0",
            LinkFlags::Up | LinkFlags::Dormant,
            Some(OperState::Dormant),
            None,
        );

        // ACT
        let parsed = parse_event(&message, Some(LinkState::Dormant));

        // ASSERT
        assert!(parsed.is_none());
    }

    #[test]
    fn parse_event_ignores_other_messages() {
        // ARRANGE
        let message = RouteNetlinkMessage::NewRoute(RouteMessage::default());

        // ACT / ASSERT
        assert!(parse_event(&message, None).is_none());
    }
}
