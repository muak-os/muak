//! Snapshot inspection shared by the supervisor's policy modules.

use linkctl::interface::Snapshot;
use linkctl::provision::State as MethodState;
use netlink::link::state::State;

/// Returns true when at least one address family has bound its configuration.
pub(super) fn is_configured(snapshot: &Snapshot) -> bool {
    #[cfg(feature = "ipv4")]
    if snapshot.v4_state == MethodState::Bound {
        return true;
    }
    #[cfg(feature = "ipv6")]
    if snapshot.v6_state == MethodState::Bound {
        return true;
    }

    false
}

/// Returns true when an address family has been started and not yet stopped.
pub(super) fn is_active(snapshot: &Snapshot) -> bool {
    #[cfg(feature = "ipv4")]
    if snapshot.v4_state != MethodState::Stopped {
        return true;
    }
    #[cfg(feature = "ipv6")]
    if snapshot.v6_state != MethodState::Stopped {
        return true;
    }

    false
}

/// Returns true when an address family is running in a degraded state.
pub(super) fn is_degraded(snapshot: &Snapshot) -> bool {
    #[cfg(feature = "ipv4")]
    if snapshot.v4_state == MethodState::Degraded {
        return true;
    }
    #[cfg(feature = "ipv6")]
    if snapshot.v6_state == MethodState::Degraded {
        return true;
    }

    false
}

/// Returns true when at least one address family still has a method view.
pub(super) const fn has_method_view(snapshot: &Snapshot) -> bool {
    #[cfg(feature = "ipv4")]
    if snapshot.v4.is_some() {
        return true;
    }
    #[cfg(feature = "ipv6")]
    if snapshot.v6.is_some() {
        return true;
    }

    false
}

/// Returns true when the interface lost its carrier or a method gave up.
pub(super) fn is_failed(snapshot: &Snapshot) -> bool {
    snapshot.link == State::Down || (is_degraded(snapshot) && !has_method_view(snapshot))
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "ipv4")]
    use alloc::vec::Vec;
    #[cfg(feature = "ipv4")]
    use core::net::Ipv4Addr;

    #[cfg(feature = "ipv4")]
    use linkctl::view::V4State;

    #[cfg(feature = "ipv4")]
    use super::*;

    #[cfg(feature = "ipv4")]
    const fn snapshot(v4: Option<V4State>, v4_state: MethodState) -> Snapshot {
        Snapshot {
            link: State::Up,
            v4,
            v4_state,
            #[cfg(feature = "ipv6")]
            v6: None,
            #[cfg(feature = "ipv6")]
            v6_state: MethodState::Stopped,
        }
    }

    #[cfg(feature = "ipv4")]
    const fn static_view() -> V4State {
        V4State::Static(netlink::address::IpConfig {
            address: Ipv4Addr::new(10, 0, 0, 2),
            prefix_len: 24,
            gateway: None,
            dns: Vec::new(),
        })
    }

    #[cfg(feature = "ipv4")]
    #[test]
    fn degraded_with_a_method_view_is_not_failed() {
        // ARRANGE
        let snapshot = snapshot(Some(static_view()), MethodState::Degraded);

        // ACT
        let failed = is_failed(&snapshot);

        // ASSERT
        assert!(!failed, "a degraded method with a view is still running");
    }

    #[cfg(feature = "ipv4")]
    #[test]
    fn degraded_without_a_method_view_is_failed() {
        // ARRANGE
        let snapshot = snapshot(None, MethodState::Degraded);

        // ACT
        let failed = is_failed(&snapshot);

        // ASSERT
        assert!(failed, "a degraded method without a view gave up");
    }
}
