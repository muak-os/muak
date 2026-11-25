//! Kernel log rendering of the observations the driver journal records.

mod detail;
mod emit;

use core::fmt;

pub(crate) use emit::emit;
use net_observe::{EffectKind, Event, LinkSnapshot};

const MAX_DISCOVERED: usize = 8;

struct Line<'a>(&'a Event);

impl fmt::Display for Line<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self.0 {
            Event::StateChanged { from, to } => write!(f, "Network state: {from} -> {to}"),
            Event::PrimaryChanged {
                ref primary,
                ref backups,
            } => {
                f.write_str("Selected primary: ")?;
                primary_name(f, primary.as_ref())?;
                f.write_str(", backups: ")?;
                name_list(f, backups)
            }
            Event::OwnerChanged { ref owner, link } => {
                f.write_str("Layer 3 owner: ")?;
                primary_name(f, owner.as_ref())?;
                f.write_str(", link: ")?;
                match link {
                    Some(state) => write!(f, "{state}"),
                    None => f.write_str("unknown"),
                }
            }
            Event::LinksDiscovered { ref links } => discovered(f, links),
            Event::LinkEvent { ref event } => link_event(f, event),
            Event::BridgeReady {
                ref name,
                index,
                state,
            } => write!(f, "Bridge {name} ready (index {index}, state: {state})"),
            Event::Enslaved {
                ref port,
                ref master,
            } => write!(f, "Port {port} enslaved to {master}"),
            Event::Detached { ref port } => write!(f, "Port {port} detached"),
            Event::EffectStarted { kind, ref target } => started(f, kind, target.as_ref()),
            Event::EffectFinished { kind, ref target } => finished(f, kind, target.as_ref()),
            Event::EffectFailed {
                kind,
                ref target,
                ref error,
            } => {
                started(f, kind, target.as_ref())?;
                write!(f, " failed: {error}")
            }
            Event::InterfaceState {
                ref name,
                family,
                state,
            } => write!(f, "{name}: {family} state {state}"),
            Event::Notice {
                ref name,
                family,
                ref text,
            } => match family {
                Some(family) => write!(f, "{name}: {family} {text}"),
                None => write!(f, "{name}: {text}"),
            },
            Event::AddressAssigned {
                ref name,
                address,
                prefix_len,
            } => write!(f, "{name}: assigned {address}/{prefix_len}"),
            Event::AddressRemoved {
                ref name,
                address,
                prefix_len,
            } => write!(f, "{name}: removed {address}/{prefix_len}"),
            Event::RouteEnsured { ref name, gateway } => {
                write!(f, "{name}: default route via {gateway}")
            }
            Event::RouteRemoved { ref name, gateway } => {
                write!(f, "{name}: default route via {gateway} removed")
            }
            Event::DhcpLease {
                ref name,
                ref lease,
            } => detail::dhcp(f, name, lease),
            Event::SlaacUpdate {
                ref name,
                ref update,
            } => detail::slaac(f, name, update),
        }
    }
}

fn discovered(formatter: &mut fmt::Formatter<'_>, links: &[LinkSnapshot]) -> fmt::Result {
    write!(formatter, "Discovered {} interface(s)", links.len())?;
    let shown = links.len().min(MAX_DISCOVERED);
    for (position, link) in links.iter().take(shown).enumerate() {
        formatter.write_str(if position == 0 { ": " } else { ", " })?;
        write!(formatter, "{}", link.name)?;
    }
    let remaining = links.len().saturating_sub(shown);
    if remaining > 0 {
        write!(formatter, ", +{remaining} more")?;
    }

    Ok(())
}

fn primary_name(formatter: &mut fmt::Formatter<'_>, name: Option<&netmodel::Name>) -> fmt::Result {
    match name {
        Some(name) => write!(formatter, "{name}"),
        None => formatter.write_str("none"),
    }
}

fn name_list(formatter: &mut fmt::Formatter<'_>, names: &[netmodel::Name]) -> fmt::Result {
    formatter.write_str("[")?;
    for (position, name) in names.iter().enumerate() {
        if position > 0 {
            formatter.write_str(", ")?;
        }
        write!(formatter, "{name}")?;
    }

    formatter.write_str("]")
}

fn link_event(formatter: &mut fmt::Formatter<'_>, event: &netmodel::LinkEvent) -> fmt::Result {
    match *event {
        netmodel::LinkEvent::Up { ref name, index } => {
            write!(formatter, "Event: Link up {name} (index {index})")
        }
        netmodel::LinkEvent::Down { ref name, index } => {
            write!(formatter, "Event: Link down {name} (index {index})")
        }
        netmodel::LinkEvent::Added {
            ref name, index, ..
        } => write!(formatter, "Event: Link added {name} (index {index})"),
        netmodel::LinkEvent::Deleted { ref name, index } => {
            write!(formatter, "Event: Link deleted {name} (index {index})")
        }
        netmodel::LinkEvent::Error(failure) => {
            write!(formatter, "Event: Link monitor error: {failure}")
        }
    }
}

fn started(
    formatter: &mut fmt::Formatter<'_>,
    kind: EffectKind,
    target: Option<&netmodel::Name>,
) -> fmt::Result {
    match (kind, target) {
        (EffectKind::Discover, _) => formatter.write_str("Discovering interfaces"),
        (EffectKind::ProbeCarriers, _) => formatter.write_str("Probing link carriers"),
        (EffectKind::SetLinkUp, Some(name)) => write!(formatter, "Bringing {name} up"),
        (EffectKind::SetLinkUp, None) => formatter.write_str("Bringing link up"),
        (EffectKind::ConfigureInterface, Some(name)) => write!(formatter, "Configuring {name}"),
        (EffectKind::ConfigureInterface, None) => formatter.write_str("Configuring interface"),
        (EffectKind::ReconcileInterface, Some(name)) => write!(formatter, "Reconciling {name}"),
        (EffectKind::ReconcileInterface, None) => formatter.write_str("Reconciling interface"),
        (EffectKind::EnsureBridge, Some(name)) => write!(formatter, "Ensuring bridge {name}"),
        (EffectKind::EnsureBridge, None) => formatter.write_str("Ensuring bridge"),
        (EffectKind::Enslave, Some(name)) => write!(formatter, "Enslaving port {name}"),
        (EffectKind::Enslave, None) => formatter.write_str("Enslaving port"),
        (EffectKind::Detach, Some(name)) => write!(formatter, "Detaching port {name}"),
        (EffectKind::Detach, None) => formatter.write_str("Detaching port"),
        (EffectKind::ShutdownInterface, Some(name)) => write!(formatter, "Shutting {name} down"),
        (EffectKind::ShutdownInterface, None) => formatter.write_str("Shutting interface down"),
        (EffectKind::Publish, _) => formatter.write_str("Publishing status"),
        (EffectKind::WriteResolver, _) => formatter.write_str("Writing resolver configuration"),
    }
}

fn finished(
    formatter: &mut fmt::Formatter<'_>,
    kind: EffectKind,
    target: Option<&netmodel::Name>,
) -> fmt::Result {
    match (kind, target) {
        (EffectKind::Discover, _) => formatter.write_str("Interface discovery completed"),
        (EffectKind::ProbeCarriers, _) => formatter.write_str("Carrier probe completed"),
        (EffectKind::SetLinkUp, Some(name)) => write!(formatter, "{name} is up"),
        (EffectKind::SetLinkUp, None) => formatter.write_str("Link is up"),
        (EffectKind::ConfigureInterface, Some(name)) => write!(formatter, "Configured {name}"),
        (EffectKind::ConfigureInterface, None) => formatter.write_str("Configured interface"),
        (EffectKind::ReconcileInterface, Some(name)) => {
            write!(formatter, "Reconcile completed on {name}")
        }
        (EffectKind::ReconcileInterface, None) => formatter.write_str("Reconcile completed"),
        (EffectKind::EnsureBridge, Some(name)) => write!(formatter, "Bridge {name} ensured"),
        (EffectKind::EnsureBridge, None) => formatter.write_str("Bridge ensured"),
        (EffectKind::Enslave, Some(name)) => write!(formatter, "Port {name} enslaved"),
        (EffectKind::Enslave, None) => formatter.write_str("Port enslaved"),
        (EffectKind::Detach, Some(name)) => write!(formatter, "Port {name} detached"),
        (EffectKind::Detach, None) => formatter.write_str("Port detached"),
        (EffectKind::ShutdownInterface, Some(name)) => {
            write!(formatter, "Interface {name} shut down")
        }
        (EffectKind::ShutdownInterface, None) => formatter.write_str("Interface shut down"),
        (EffectKind::Publish, _) => formatter.write_str("Status published"),
        (EffectKind::WriteResolver, _) => formatter.write_str("Resolver configuration written"),
    }
}

#[cfg(test)]
mod tests {
    use core::fmt::Write as _;
    use core::num::NonZeroI32;

    use net_observe::{EffectKind, Event, SupervisorState, interface};

    use super::Line;

    fn render(event: &Event) -> String {
        let mut output = String::new();
        write!(output, "{}", Line(event)).expect("render line");

        output
    }

    fn name(value: &str) -> netmodel::Name {
        netmodel::Name::new(value).expect("valid name")
    }

    #[test]
    fn state_change_reads_as_a_transition() {
        // ARRANGE
        let event = Event::StateChanged {
            from: SupervisorState::Initializing,
            to: SupervisorState::Operational,
        };

        // ACT
        let text = render(&event);

        // ASSERT
        assert_eq!(text, "Network state: Initializing -> Operational");
    }

    #[test]
    fn primary_change_lists_the_backups() {
        // ARRANGE
        let event = Event::PrimaryChanged {
            primary: Some(name("eth0")),
            backups: vec![name("eth1")],
        };

        // ACT
        let text = render(&event);

        // ASSERT
        assert_eq!(text, "Selected primary: eth0, backups: [eth1]");
    }

    #[test]
    fn owner_change_includes_the_link_state() {
        // ARRANGE
        let event = Event::OwnerChanged {
            owner: Some(name("br0")),
            link: Some(netmodel::LinkState::Up),
        };

        // ACT
        let text = render(&event);

        // ASSERT
        assert_eq!(text, "Layer 3 owner: br0, link: up");
    }

    #[test]
    fn discovered_links_are_counted_and_listed() {
        // ARRANGE
        let physical = netmodel::Link {
            name: name("eth0"),
            index: 8,
            mac: netmodel::Mac::default(),
            admin: netmodel::AdminState::Up,
            carrier: netmodel::CarrierState::Up,
            is_virtual: false,
            master: None,
        };
        let event = Event::LinksDiscovered {
            links: vec![net_observe::LinkSnapshot::from(&physical)],
        };

        // ACT
        let text = render(&event);

        // ASSERT
        assert_eq!(text, "Discovered 1 interface(s): eth0");
    }

    #[test]
    fn discovered_links_are_truncated_after_the_cap() {
        // ARRANGE
        let physical = netmodel::Link {
            name: name("eth0"),
            index: 8,
            mac: netmodel::Mac::default(),
            admin: netmodel::AdminState::Up,
            carrier: netmodel::CarrierState::Up,
            is_virtual: false,
            master: None,
        };
        let links = (0_usize..10)
            .map(|index| {
                net_observe::LinkSnapshot::from(&netmodel::Link {
                    name: name(&format!("eth{index}")),
                    ..physical.clone()
                })
            })
            .collect();
        let event = Event::LinksDiscovered { links };

        // ACT
        let text = render(&event);

        // ASSERT
        assert_eq!(
            text,
            "Discovered 10 interface(s): eth0, eth1, eth2, eth3, eth4, eth5, eth6, eth7, +2 more"
        );
        assert!(text.len() <= 992);
    }

    #[test]
    fn link_events_name_the_link_and_index() {
        // ARRANGE
        let event = Event::LinkEvent {
            event: netmodel::LinkEvent::Up {
                name: name("eth0"),
                index: 8,
            },
        };

        // ACT
        let text = render(&event);

        // ASSERT
        assert_eq!(text, "Event: Link up eth0 (index 8)");
    }

    #[test]
    fn monitor_failures_report_the_cause() {
        // ARRANGE
        let event = Event::LinkEvent {
            event: netmodel::LinkEvent::Error(netmodel::LinkFailure::Kernel(
                NonZeroI32::new(105_i32).expect("non-zero"),
            )),
        };

        // ACT
        let text = render(&event);

        // ASSERT
        assert_eq!(
            text,
            "Event: Link monitor error: netlink event socket error: 105"
        );
    }

    #[test]
    fn bridge_and_port_events_read_naturally() {
        // ACT / ASSERT
        assert_eq!(
            render(&Event::BridgeReady {
                name: name("br0"),
                index: 9,
                state: netmodel::LinkState::Up,
            }),
            "Bridge br0 ready (index 9, state: up)"
        );
        assert_eq!(
            render(&Event::Enslaved {
                port: name("eth0"),
                master: name("br0"),
            }),
            "Port eth0 enslaved to br0"
        );
        assert_eq!(
            render(&Event::Detached { port: name("eth0") }),
            "Port eth0 detached"
        );
    }

    #[test]
    fn configured_effects_report_target_and_outcome() {
        // ARRANGE
        let target = Some(name("br0"));

        // ACT / ASSERT
        assert_eq!(
            render(&Event::EffectStarted {
                kind: EffectKind::ConfigureInterface,
                target: target.clone(),
            }),
            "Configuring br0"
        );
        assert_eq!(
            render(&Event::EffectFinished {
                kind: EffectKind::ConfigureInterface,
                target: target.clone(),
            }),
            "Configured br0"
        );
        assert_eq!(
            render(&Event::EffectFailed {
                kind: EffectKind::ConfigureInterface,
                target,
                error: "no such device".to_owned(),
            }),
            "Configuring br0 failed: no such device"
        );
    }

    #[test]
    fn interface_state_and_notices_name_the_family() {
        // ACT / ASSERT
        assert_eq!(
            render(&Event::InterfaceState {
                name: name("eth0"),
                family: interface::Family::V4,
                state: interface::MethodState::Acquiring,
            }),
            "eth0: IPv4 state acquiring"
        );
        assert_eq!(
            render(&Event::Notice {
                name: name("eth0"),
                family: None,
                text: "no carrier".to_owned(),
            }),
            "eth0: no carrier"
        );
        assert_eq!(
            render(&Event::Notice {
                name: name("eth0"),
                family: Some(interface::Family::V4),
                text: "lease renew failed".to_owned(),
            }),
            "eth0: IPv4 lease renew failed"
        );
    }

    #[test]
    fn address_and_route_lines_name_the_address() {
        // ARRANGE
        let address = core::net::IpAddr::V4(core::net::Ipv4Addr::new(10, 0, 0, 2));
        let gateway = core::net::IpAddr::V4(core::net::Ipv4Addr::new(10, 0, 0, 1));

        // ACT / ASSERT
        assert_eq!(
            render(&Event::AddressAssigned {
                name: name("eth0"),
                address,
                prefix_len: 24,
            }),
            "eth0: assigned 10.0.0.2/24"
        );
        assert_eq!(
            render(&Event::AddressRemoved {
                name: name("eth0"),
                address,
                prefix_len: 24,
            }),
            "eth0: removed 10.0.0.2/24"
        );
        assert_eq!(
            render(&Event::RouteEnsured {
                name: name("eth0"),
                gateway,
            }),
            "eth0: default route via 10.0.0.1"
        );
        assert_eq!(
            render(&Event::RouteRemoved {
                name: name("eth0"),
                gateway,
            }),
            "eth0: default route via 10.0.0.1 removed"
        );
    }

    #[test]
    fn dhcp_lease_reports_every_negotiated_detail() {
        // ARRANGE
        let lease = interface::DhcpLease {
            address: core::net::Ipv4Addr::new(10, 0, 0, 2),
            prefix_len: 24,
            server: core::net::Ipv4Addr::new(10, 0, 0, 1),
            gateway: Some(core::net::Ipv4Addr::new(10, 0, 0, 1)),
            dns: vec![core::net::Ipv4Addr::new(8, 8, 8, 8)],
            phase: interface::DhcpPhase::Bound,
        };

        // ACT
        let text = render(&Event::DhcpLease {
            name: name("eth0"),
            lease,
        });

        // ASSERT
        assert_eq!(
            text,
            "eth0: IPv4 dhcp 10.0.0.2/24 server 10.0.0.1 gateway 10.0.0.1 dns [8.8.8.8] (bound)"
        );
    }

    #[test]
    fn dhcp_phase_is_appended_and_dns_is_truncated() {
        // ARRANGE
        let dns = (0_usize..10)
            .map(|index| {
                let octet = u8::try_from(index + 1).expect("small index");
                core::net::Ipv4Addr::new(9, 9, 9, octet)
            })
            .collect();
        let lease = interface::DhcpLease {
            address: core::net::Ipv4Addr::new(10, 0, 0, 2),
            prefix_len: 24,
            server: core::net::Ipv4Addr::new(10, 0, 0, 1),
            gateway: None,
            dns,
            phase: interface::DhcpPhase::Degraded,
        };

        // ACT
        let text = render(&Event::DhcpLease {
            name: name("eth0"),
            lease,
        });

        // ASSERT
        assert_eq!(
            text,
            "eth0: IPv4 dhcp 10.0.0.2/24 server 10.0.0.1 gateway none dns \
             [9.9.9.1, 9.9.9.2, 9.9.9.3, 9.9.9.4, 9.9.9.5, 9.9.9.6, 9.9.9.7, 9.9.9.8, +2 more] \
             (degraded)"
        );
    }

    #[test]
    fn slaac_update_lists_addresses_routers_and_dns() {
        // ARRANGE
        let update = interface::SlaacUpdate {
            addresses: vec![(
                core::net::Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0, 0, 0, 2),
                64,
            )],
            routers: vec![core::net::Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1)],
            dns: Vec::new(),
        };

        // ACT
        let text = render(&Event::SlaacUpdate {
            name: name("eth0"),
            update,
        });

        // ASSERT
        assert_eq!(
            text,
            "eth0: IPv6 slaac [2001:db8::2/64] routers [fe80::1] dns []"
        );
    }

    #[test]
    fn untargeted_effects_have_a_fallback() {
        // ACT / ASSERT
        assert_eq!(
            render(&Event::EffectStarted {
                kind: EffectKind::Discover,
                target: None,
            }),
            "Discovering interfaces"
        );
        assert_eq!(
            render(&Event::EffectFinished {
                kind: EffectKind::Publish,
                target: None,
            }),
            "Status published"
        );
    }
}
