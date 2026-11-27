//! Link snapshot integration tests.

pub mod common;

#[cfg(test)]
mod tests {
    use netmodel::{AdminState, CarrierState, LinkState};

    use crate::common;

    #[test]
    fn has_carrier_follows_state() {
        // ARRANGE
        let up = common::link("eth0", LinkState::Up);
        let down = common::link("eth0", LinkState::Down);

        // ACT / ASSERT
        assert!(up.has_carrier());
        assert!(!down.has_carrier());
    }

    #[test]
    fn dormant_and_no_carrier_have_no_carrier() {
        // ARRANGE
        let dormant = common::link("eth0", LinkState::Dormant);
        let no_carrier = common::link("eth0", LinkState::NoCarrier);

        // ACT / ASSERT
        assert!(!dormant.has_carrier());
        assert!(!no_carrier.has_carrier());
    }

    #[test]
    fn unknown_carrier_while_admin_up_is_usable() {
        // ARRANGE
        let link = common::link_with_parts("eth0", 2, AdminState::Up, CarrierState::Unknown);

        // ACT
        let state = link.state();

        // ASSERT
        assert_eq!(state, LinkState::Up);
        assert!(link.has_carrier());
    }

    #[test]
    fn admin_state_is_reported_independently_of_carrier() {
        // ARRANGE
        let link = common::link_with_parts("eth0", 2, AdminState::Up, CarrierState::Down);

        // ACT / ASSERT
        assert!(link.is_admin_up());
        assert!(!link.has_carrier());
        assert_eq!(link.state(), LinkState::NoCarrier);
    }

    #[test]
    fn is_ethernet_accepts_wired_names() {
        // ARRANGE
        let names = ["eth0", "enp3s0f0", "ens18", "eno1", "end0"];

        // ACT / ASSERT
        for name in names {
            assert!(common::link(name, LinkState::Up).is_ethernet(), "{name}");
        }
    }

    #[test]
    fn is_ethernet_rejects_loopback_and_wireless_names() {
        // ARRANGE
        let names = ["lo", "wlan0", "wlp2s0", "veth0", "docker0"];

        // ACT / ASSERT
        for name in names {
            assert!(!common::link(name, LinkState::Up).is_ethernet(), "{name}");
        }
    }
}
