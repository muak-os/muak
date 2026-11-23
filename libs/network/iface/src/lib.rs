//! Per-interface network configuration finite state machine.

#![warn(missing_docs)]

use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::{Instant, SystemTime};

/// Address family the controller drives for one interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// IPv4.
    V4,
    /// IPv6.
    V6,
}

/// Commands the supervisor sends to an interface controller.
#[derive(Debug, Clone)]
pub enum Command {
    /// Apply the given per-family configurations.
    Configure {
        /// IPv4 configuration to apply.
        v4: Option<netmodel::Ipv4Config>,
        /// IPv6 configuration to apply.
        v6: Option<netmodel::Ipv6Config>,
    },
    /// Reconcile desired and actual configuration.
    Reconcile,
    /// Bring the link administratively up.
    LinkUp,
    /// Bring the link administratively down.
    LinkDown,
    /// Stop every method and shut the controller down.
    Shutdown,
}

/// Input an interface controller consumes on each step.
#[derive(Debug)]
pub enum Input<'a> {
    /// Poll the controller without new external input.
    Poll,
    /// Supervisor command.
    Command(Command),
    /// Link snapshot update.
    Link(ip_method::LinkInfo),
    /// Input for one address family method.
    Method {
        /// Family the input belongs to.
        family: Family,
        /// Input to hand to the family method.
        input: ip_method::Input<'a>,
    },
    /// Timer deadline elapsed.
    Timer,
}

/// Effect an interface controller requests from the runtime.
#[derive(Debug)]
pub enum Effect<'a> {
    /// Bring the link administratively up.
    SetLinkUp {
        /// Link to bring up.
        link: netmodel::Name,
    },
    /// Bring the link administratively down.
    SetLinkDown {
        /// Link to bring down.
        link: netmodel::Name,
    },
    /// Assign an address to the link.
    AddAddress {
        /// Link to assign the address to.
        link: netmodel::Name,
        /// Address to assign.
        address: IpAddr,
        /// Prefix length in bits.
        prefix_len: u8,
    },
    /// Remove an address from the link.
    RemoveAddress {
        /// Link to remove the address from.
        link: netmodel::Name,
        /// Address to remove.
        address: IpAddr,
        /// Prefix length in bits, the address port needs the exact pair.
        prefix_len: u8,
    },
    /// Ensure a default route through the gateway.
    EnsureRoute {
        /// Link the route goes through.
        link: netmodel::Name,
        /// Gateway of the default route.
        gateway: IpAddr,
    },
    /// Remove the default route through the gateway.
    RemoveRoute {
        /// Link the route goes through.
        link: netmodel::Name,
        /// Gateway of the default route.
        gateway: IpAddr,
    },
    /// Open a socket for the family.
    OpenSocket {
        /// Family the socket belongs to.
        family: Family,
        /// Specification of the socket to open.
        spec: ip_method::SocketSpec,
    },
    /// Send a frame on a socket the family already opened.
    ///
    /// `dst` is `None` for raw layer 2 and `UDP` sockets, whose destination
    /// comes from the frame and the socket peer, and required for raw
    /// `ICMPv6` sockets.
    Send {
        /// Family the socket belongs to.
        family: Family,
        /// Slot of the socket to send on.
        slot: ip_method::SocketSlot,
        /// Raw frame bytes to send.
        frame: &'a [u8],
        /// Destination address, required only for `ICMPv6` sockets.
        dst: Option<core::net::SocketAddr>,
    },
    /// Close a socket the family opened.
    CloseSocket {
        /// Family the socket belongs to.
        family: Family,
        /// Slot of the socket to close.
        slot: ip_method::SocketSlot,
    },
    /// Publish the interface snapshot.
    Publish,
}

/// Outcome of one interface controller step.
#[derive(Debug)]
pub enum Step<'a> {
    /// The controller produced an effect for the runtime.
    Effect(Effect<'a>),
    /// The controller is waiting for input or for its wake deadline.
    Wait,
}

/// IPv4 method view exposed to the supervisor.
#[derive(Debug, Clone)]
pub enum V4View {
    /// Static address configuration.
    Static(ip_static::View<Ipv4Addr>),
}

/// IPv6 method view exposed to the supervisor.
#[derive(Debug, Clone)]
pub enum V6View {
    /// Static address configuration.
    Static(ip_static::View<Ipv6Addr>),
}

/// Point-in-time state of an interface and its address methods.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Administrative and carrier state of the link.
    pub link: netmodel::LinkState,
    /// IPv4 method view, when configured.
    pub v4: Option<V4View>,
    /// IPv4 provisioning phase.
    pub v4_state: ip_method::State,
    /// IPv6 method view, when configured.
    pub v6: Option<V6View>,
    /// IPv6 provisioning phase.
    pub v6_state: ip_method::State,
}

/// Family-neutral controller for one interface.
#[derive(Debug)]
pub struct Controller {
    link: ip_method::LinkInfo,
    snapshot: Snapshot,
}

impl Controller {
    /// Creates a controller for the given link with an idle snapshot.
    #[must_use = "the controller owns the interface state"]
    pub const fn new(link: ip_method::LinkInfo) -> Self {
        let snapshot = Snapshot {
            link: link.state,
            v4: None,
            v4_state: ip_method::State::Stopped,
            v6: None,
            v6_state: ip_method::State::Stopped,
        };

        Self { link, snapshot }
    }

    /// Performs at most one step and returns the next effect.
    pub fn step(&mut self, _now: Instant, _wall: SystemTime, _input: Input<'_>) -> Step<'_> {
        Step::Wait
    }

    /// Earliest instant at which the controller must be stepped again.
    #[must_use = "the caller needs the deadline to schedule the wake"]
    pub const fn next_wake(&self, _now: Instant) -> Option<Instant> {
        None
    }

    /// Returns the controller's current snapshot.
    #[must_use = "callers need the snapshot to report interface state"]
    pub const fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    /// Returns the link the controller runs on.
    #[must_use = "callers need the link to address interface effects"]
    pub const fn link(&self) -> &ip_method::LinkInfo {
        &self.link
    }
}

#[cfg(test)]
mod tests {
    use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use std::time::{Instant, SystemTime};

    use netmodel::{LinkState, Mac, Name};

    use crate::{Command, Controller, Effect, Family, Input, Snapshot, Step, V4View};

    fn link() -> ip_method::LinkInfo {
        ip_method::LinkInfo {
            name: Name::new("eth0").expect("valid name"),
            index: 2,
            mac: Mac::default(),
            state: LinkState::Up,
        }
    }

    #[test]
    fn new_snapshots_the_link_state() {
        // ARRANGE
        let link = link();

        // ACT
        let controller = Controller::new(link);

        // ASSERT
        let snapshot = controller.snapshot();
        assert_eq!(snapshot.link, LinkState::Up);
        assert!(snapshot.v4.is_none());
        assert!(snapshot.v6.is_none());
        assert_eq!(snapshot.v4_state, ip_method::State::Stopped);
        assert_eq!(snapshot.v6_state, ip_method::State::Stopped);
    }

    #[test]
    fn link_returns_the_controller_link() {
        // ARRANGE
        let controller = Controller::new(link());

        // ACT
        let stored = controller.link();

        // ASSERT
        assert_eq!(stored.name, "eth0");
        assert_eq!(stored.index, 2);
        assert_eq!(stored.state, LinkState::Up);
    }

    #[test]
    fn step_waits_and_has_no_wake() {
        // ARRANGE
        let mut controller = Controller::new(link());
        let now = Instant::now();

        // ACT
        let waits = matches!(
            controller.step(now, SystemTime::UNIX_EPOCH, Input::Poll),
            Step::Wait
        );

        // ASSERT
        assert!(waits);
        assert!(controller.next_wake(now).is_none());
    }

    #[test]
    fn commands_can_be_constructed() {
        // ARRANGE
        let v4 = netmodel::Ipv4Config {
            addresses: vec![(Ipv4Addr::new(10, 0, 0, 2), 24)],
            ..netmodel::Ipv4Config::default()
        };
        let v6 = netmodel::Ipv6Config {
            addresses: vec![(Ipv6Addr::LOCALHOST, 128)],
            ..netmodel::Ipv6Config::default()
        };

        // ACT
        let configure = Command::Configure {
            v4: Some(v4),
            v6: Some(v6),
        };
        let Command::Configure { v4, v6 } = configure else {
            panic!("configure variant expected");
        };

        // ASSERT
        assert!(v4.is_some_and(|config| config.addresses.len() == 1));
        assert!(v6.is_some_and(|config| config.addresses.len() == 1));
    }

    #[test]
    fn unit_commands_can_be_constructed() {
        // ARRANGE
        let commands = [
            Command::Reconcile,
            Command::LinkUp,
            Command::LinkDown,
            Command::Shutdown,
        ];

        // ACT
        let each = commands
            .iter()
            .map(|command| !matches!(command, Command::Configure { .. }));

        // ASSERT
        assert!(each.into_iter().all(core::convert::identity));
    }

    #[test]
    fn method_input_and_effects_carry_the_family() {
        // ARRANGE
        let frame = [0_u8; 4];

        // ACT
        let input = Input::Method {
            family: Family::V6,
            input: ip_method::Input::Poll,
        };
        let effect = Effect::Send {
            family: Family::V6,
            slot: ip_method::SocketSlot::Primary,
            frame: &frame,
            dst: None,
        };

        // ASSERT
        assert!(matches!(
            input,
            Input::Method {
                family: Family::V6,
                ..
            }
        ));
        assert!(matches!(
            effect,
            Effect::Send {
                family: Family::V6,
                ..
            }
        ));
    }

    #[test]
    fn views_expose_static_configuration() {
        // ARRANGE
        let view = ip_static::View {
            address: Ipv4Addr::new(10, 0, 0, 2),
            prefix_len: 24,
            gateway: Some(Ipv4Addr::new(10, 0, 0, 1)),
            dns: vec![Ipv4Addr::new(10, 0, 0, 53)],
        };

        // ACT
        let view = V4View::Static(view);
        let snapshot = Snapshot {
            link: LinkState::Up,
            v4: Some(view),
            v4_state: ip_method::State::Bound,
            v6: None,
            v6_state: ip_method::State::Stopped,
        };

        // ASSERT
        let Some(V4View::Static(config)) = snapshot.v4 else {
            panic!("static view expected");
        };
        assert_eq!(config.address, Ipv4Addr::new(10, 0, 0, 2));
        assert_eq!(config.prefix_len, 24);
        assert_eq!(
            config.gateway.map(IpAddr::V4),
            Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)))
        );
    }
}
