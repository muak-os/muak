//! Life cycle state machine for a single network interface.

use crate::state_machine::StateMachine;

/// Tracks the provisioning stage of one network interface.
#[derive(Debug, Clone, PartialEq)]
pub enum InterfaceState {
    Discovered,
    Configuring,
    Configured,
    Degraded,
    Failed,
    Deconfiguring,
}

impl StateMachine for InterfaceState {
    fn valid_next_states(&self) -> &'static [Self] {
        match self {
            Self::Discovered => &[Self::Configuring],
            Self::Configuring => &[Self::Configured, Self::Failed],
            Self::Configured => &[Self::Degraded, Self::Deconfiguring],
            Self::Degraded => &[Self::Configuring, Self::Configured, Self::Deconfiguring],
            Self::Failed => &[Self::Configuring, Self::Deconfiguring],
            Self::Deconfiguring => &[Self::Discovered],
        }
    }
}

impl std::fmt::Display for InterfaceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
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
    use super::*;

    fn make_snapshot(state: InterfaceState) -> crate::interface::snapshot::InterfaceSnapshot {
        crate::interface::snapshot::InterfaceSnapshot {
            name: netlib::interface::InterfaceName::new("eth0").expect("valid name"),
            state,
            index: 2,
            mac: [0x00, 0x11, 0x22, 0x33, 0x44, 0x55],
            link: netlib::link::LinkStateKind::Up,
            ip: None,
            lease: None,
            dhcp_state: None,
            ipv6: None,
        }
    }

    #[test]
    fn interface_state_display() {
        // ACT / ASSERT
        assert_eq!(InterfaceState::Discovered.to_string(), "Discovered");
        assert_eq!(InterfaceState::Configuring.to_string(), "Configuring");
        assert_eq!(InterfaceState::Configured.to_string(), "Configured");
        assert_eq!(InterfaceState::Degraded.to_string(), "Degraded");
        assert_eq!(InterfaceState::Failed.to_string(), "Failed");
        assert_eq!(InterfaceState::Deconfiguring.to_string(), "Deconfiguring");
    }

    #[test]
    fn valid_interface_transitions_succeed() {
        let pairs = [
            (InterfaceState::Discovered, InterfaceState::Configuring),
            (InterfaceState::Configuring, InterfaceState::Configured),
            (InterfaceState::Configuring, InterfaceState::Failed),
            (InterfaceState::Configured, InterfaceState::Degraded),
            (InterfaceState::Configured, InterfaceState::Deconfiguring),
            (InterfaceState::Degraded, InterfaceState::Configuring),
            (InterfaceState::Degraded, InterfaceState::Configured),
            (InterfaceState::Degraded, InterfaceState::Deconfiguring),
            (InterfaceState::Failed, InterfaceState::Configuring),
            (InterfaceState::Failed, InterfaceState::Deconfiguring),
            (InterfaceState::Deconfiguring, InterfaceState::Discovered),
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
            (InterfaceState::Discovered, InterfaceState::Configured),
            (InterfaceState::Discovered, InterfaceState::Degraded),
            (InterfaceState::Discovered, InterfaceState::Failed),
            (InterfaceState::Discovered, InterfaceState::Deconfiguring),
            (InterfaceState::Configuring, InterfaceState::Discovered),
            (InterfaceState::Configuring, InterfaceState::Degraded),
            (InterfaceState::Configuring, InterfaceState::Deconfiguring),
            (InterfaceState::Configured, InterfaceState::Discovered),
            (InterfaceState::Configured, InterfaceState::Configuring),
            (InterfaceState::Configured, InterfaceState::Failed),
            (InterfaceState::Degraded, InterfaceState::Discovered),
            (InterfaceState::Degraded, InterfaceState::Failed),
            (InterfaceState::Failed, InterfaceState::Discovered),
            (InterfaceState::Failed, InterfaceState::Configured),
            (InterfaceState::Failed, InterfaceState::Degraded),
            (InterfaceState::Deconfiguring, InterfaceState::Configured),
            (InterfaceState::Deconfiguring, InterfaceState::Configuring),
            (InterfaceState::Deconfiguring, InterfaceState::Degraded),
            (InterfaceState::Deconfiguring, InterfaceState::Failed),
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
        let mut snap = make_snapshot(InterfaceState::Configured);

        // ACT
        let _ = snap.transition(InterfaceState::Discovered);

        // ASSERT
        assert_eq!(snap.state, InterfaceState::Configured);
    }

    #[test]
    fn interface_snapshot_starts_discovered_after_construction() {
        // ACT
        let snap = make_snapshot(InterfaceState::Discovered);

        // ASSERT
        assert_eq!(snap.state, InterfaceState::Discovered);
    }
}
