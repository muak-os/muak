//! Life cycle state machine for a single network interface.

use core::fmt;

use crate::statemachine::StateMachine;

/// Tracks the provisioning stage of one network interface.
#[derive(Debug, Clone, PartialEq)]
pub enum Lifecycle {
    Discovered,
    Configuring,
    Configured,
    Degraded,
    Failed,
    Deconfiguring,
}

impl StateMachine for Lifecycle {
    fn valid_next_states(&self) -> &'static [Self] {
        match *self {
            Self::Discovered => &[Self::Configuring],
            Self::Configuring => &[Self::Configured, Self::Failed],
            Self::Configured => &[Self::Degraded, Self::Deconfiguring],
            Self::Degraded => &[Self::Configuring, Self::Configured, Self::Deconfiguring],
            Self::Failed => &[Self::Configuring, Self::Deconfiguring],
            Self::Deconfiguring => &[Self::Discovered],
        }
    }
}

impl fmt::Display for Lifecycle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match *self {
            Self::Discovered => "Discovered",
            Self::Configuring => "Configuring",
            Self::Configured => "Configured",
            Self::Degraded => "Degraded",
            Self::Failed => "Failed",
            Self::Deconfiguring => "Deconfiguring",
        })
    }
}

#[cfg(test)]
mod tests {
    use netlib::interface::Name;
    use netlib::link::State;

    use super::*;
    use crate::interface::snapshot::Snapshot;

    fn make_snapshot(state: Lifecycle) -> Snapshot {
        Snapshot {
            name: Name::new("eth0").expect("valid name"),
            state,
            index: 2,
            mac: [0x00, 0x11, 0x22, 0x33, 0x44, 0x55],
            link: State::Up,
            ip: None,
            lease: None,
            dhcp_state: None,
            ipv6: None,
            l3_owner: Name::new("eth0").expect("valid name"),
        }
    }

    #[test]
    fn interface_state_display() {
        // ACT / ASSERT
        assert_eq!(Lifecycle::Discovered.to_string(), "Discovered");
        assert_eq!(Lifecycle::Configuring.to_string(), "Configuring");
        assert_eq!(Lifecycle::Configured.to_string(), "Configured");
        assert_eq!(Lifecycle::Degraded.to_string(), "Degraded");
        assert_eq!(Lifecycle::Failed.to_string(), "Failed");
        assert_eq!(Lifecycle::Deconfiguring.to_string(), "Deconfiguring");
    }

    #[test]
    fn valid_interface_transitions_succeed() {
        let pairs = [
            (Lifecycle::Discovered, Lifecycle::Configuring),
            (Lifecycle::Configuring, Lifecycle::Configured),
            (Lifecycle::Configuring, Lifecycle::Failed),
            (Lifecycle::Configured, Lifecycle::Degraded),
            (Lifecycle::Configured, Lifecycle::Deconfiguring),
            (Lifecycle::Degraded, Lifecycle::Configuring),
            (Lifecycle::Degraded, Lifecycle::Configured),
            (Lifecycle::Degraded, Lifecycle::Deconfiguring),
            (Lifecycle::Failed, Lifecycle::Configuring),
            (Lifecycle::Failed, Lifecycle::Deconfiguring),
            (Lifecycle::Deconfiguring, Lifecycle::Discovered),
        ];

        for (from, to) in pairs {
            // ARRANGE
            let mut snap = make_snapshot(from.clone());

            // ACT
            let result = snap.transition(to.clone());

            // ASSERT
            assert!(result.is_ok(), "{from} -> {to} should be valid");
            assert_eq!(snap.state, to);
        }
    }

    #[test]
    fn invalid_interface_transitions_return_error() {
        let pairs = [
            (Lifecycle::Discovered, Lifecycle::Configured),
            (Lifecycle::Discovered, Lifecycle::Degraded),
            (Lifecycle::Discovered, Lifecycle::Failed),
            (Lifecycle::Discovered, Lifecycle::Deconfiguring),
            (Lifecycle::Configuring, Lifecycle::Discovered),
            (Lifecycle::Configuring, Lifecycle::Degraded),
            (Lifecycle::Configuring, Lifecycle::Deconfiguring),
            (Lifecycle::Configured, Lifecycle::Discovered),
            (Lifecycle::Configured, Lifecycle::Configuring),
            (Lifecycle::Configured, Lifecycle::Failed),
            (Lifecycle::Degraded, Lifecycle::Discovered),
            (Lifecycle::Degraded, Lifecycle::Failed),
            (Lifecycle::Failed, Lifecycle::Discovered),
            (Lifecycle::Failed, Lifecycle::Configured),
            (Lifecycle::Failed, Lifecycle::Degraded),
            (Lifecycle::Deconfiguring, Lifecycle::Configured),
            (Lifecycle::Deconfiguring, Lifecycle::Configuring),
            (Lifecycle::Deconfiguring, Lifecycle::Degraded),
            (Lifecycle::Deconfiguring, Lifecycle::Failed),
        ];

        for (from, to) in pairs {
            // ARRANGE
            let mut snap = make_snapshot(from.clone());

            // ACT
            let result = snap.transition(to.clone());

            // ASSERT
            assert!(result.is_err(), "{from} -> {to} should be invalid");
            assert_eq!(
                snap.state, from,
                "state must not change on invalid transition"
            );
        }
    }

    #[test]
    fn transition_does_not_mutate_state_on_error() {
        // ARRANGE
        let mut snap = make_snapshot(Lifecycle::Configured);

        // ACT
        let _result = snap.transition(Lifecycle::Discovered);

        // ASSERT
        assert_eq!(snap.state, Lifecycle::Configured);
    }

    #[test]
    fn interface_snapshot_starts_discovered_after_construction() {
        // ACT
        let snap = make_snapshot(Lifecycle::Discovered);

        // ASSERT
        assert_eq!(snap.state, Lifecycle::Discovered);
    }
}
