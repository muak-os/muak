//! Link snapshot integration tests.

pub mod common;

#[cfg(test)]
mod tests {
    use netmodel::LinkState;

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
