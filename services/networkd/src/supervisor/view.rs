//! Snapshot inspection shared by the supervisor's policy modules.

use addrconf::State as MethodState;
use linkctl::family::Family as _;
use linkctl::interface::Snapshot;
use linkctl::system::{V4, V6};
use netlink::link::state::State;

/// Returns true when at least one address family has bound its configuration.
pub(super) fn is_configured(snapshot: &Snapshot) -> bool {
    (V4::ENABLED && snapshot.v4_state == MethodState::Bound)
        || (V6::ENABLED && snapshot.v6_state == MethodState::Bound)
}

/// Returns true when an address family has been started and not yet stopped.
pub(super) fn is_active(snapshot: &Snapshot) -> bool {
    (V4::ENABLED && snapshot.v4_state != MethodState::Stopped)
        || (V6::ENABLED && snapshot.v6_state != MethodState::Stopped)
}

/// Returns true when an address family is running in a degraded state.
pub(super) fn is_degraded(snapshot: &Snapshot) -> bool {
    (V4::ENABLED && snapshot.v4_state == MethodState::Degraded)
        || (V6::ENABLED && snapshot.v6_state == MethodState::Degraded)
}

/// Returns true when at least one address family still has a method view.
pub(super) const fn has_method_view(snapshot: &Snapshot) -> bool {
    (V4::ENABLED && snapshot.v4.is_some()) || (V6::ENABLED && snapshot.v6.is_some())
}

/// Returns true when the interface lost its carrier or a method gave up.
pub(super) fn is_failed(snapshot: &Snapshot) -> bool {
    snapshot.link == State::Down || (is_degraded(snapshot) && !has_method_view(snapshot))
}

#[cfg(test)]
mod tests {
    #[cfg(any(feature = "ipv4", feature = "ipv6"))]
    use alloc::vec::Vec;
    #[cfg(feature = "ipv4")]
    use core::net::Ipv4Addr;
    #[cfg(feature = "ipv6")]
    use core::net::Ipv6Addr;

    #[cfg(feature = "ipv4")]
    use linkctl::view::V4State;
    #[cfg(feature = "ipv6")]
    use linkctl::view::V6State;

    #[cfg(any(feature = "ipv4", feature = "ipv6"))]
    use super::*;

    #[cfg(feature = "ipv4")]
    const fn snapshot(v4: Option<V4State>, v4_state: MethodState) -> Snapshot {
        Snapshot {
            link: State::Up,
            v4,
            v4_state,
            v6: None,
            v6_state: MethodState::Stopped,
        }
    }

    #[cfg(feature = "ipv4")]
    const fn static_view() -> V4State {
        V4State::Static(addrconf::fixed::StaticView {
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

    #[cfg(feature = "ipv6")]
    const fn snapshot_v6(v6: Option<V6State>, v6_state: MethodState) -> Snapshot {
        Snapshot {
            link: State::Up,
            v4: None,
            v4_state: MethodState::Stopped,
            v6,
            v6_state,
        }
    }

    #[cfg(feature = "ipv6")]
    const fn static_view_v6() -> V6State {
        V6State::Static(addrconf::fixed::StaticView {
            address: Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 2),
            prefix_len: 64,
            gateway: None,
            dns: Vec::new(),
        })
    }

    #[cfg(feature = "ipv6")]
    #[test]
    fn v6_bound_state_is_configured() {
        // ARRANGE
        let snapshot = snapshot_v6(None, MethodState::Bound);

        // ACT
        let configured = is_configured(&snapshot);

        // ASSERT
        assert!(configured, "the v6 method bound without exposing a view");
    }

    #[cfg(feature = "ipv6")]
    #[test]
    fn v6_degraded_state_is_degraded() {
        // ARRANGE
        let snapshot = snapshot_v6(None, MethodState::Degraded);

        // ACT
        let degraded = is_degraded(&snapshot);

        // ASSERT
        assert!(degraded, "the v6 method reported a degraded state");
    }

    #[cfg(feature = "ipv6")]
    #[test]
    fn v6_method_view_is_present() {
        // ARRANGE
        let snapshot = snapshot_v6(Some(static_view_v6()), MethodState::Bound);

        // ACT
        let present = has_method_view(&snapshot);

        // ASSERT
        assert!(present, "the v6 method exposed a static view");
    }

    #[cfg(feature = "ipv6")]
    #[test]
    fn v6_degraded_with_a_method_view_is_not_failed() {
        // ARRANGE
        let snapshot = snapshot_v6(Some(static_view_v6()), MethodState::Degraded);

        // ACT
        let failed = is_failed(&snapshot);

        // ASSERT
        assert!(!failed, "a degraded v6 method with a view is still running");
    }
}
