//! Snapshot inspection shared by the supervisor's policy modules.

use netmodel::MethodState;

/// Returns true when at least one enabled address family has bound its configuration.
pub(crate) fn is_configured(snapshot: &iface::Snapshot) -> bool {
    (cfg!(feature = "ipv4") && snapshot.v4_state == MethodState::Bound)
        || (cfg!(feature = "ipv6") && snapshot.v6_state == MethodState::Bound)
}

/// Returns true when an address family has been started and not yet stopped.
pub(crate) fn is_active(snapshot: &iface::Snapshot) -> bool {
    (cfg!(feature = "ipv4") && snapshot.v4_state != MethodState::Stopped)
        || (cfg!(feature = "ipv6") && snapshot.v6_state != MethodState::Stopped)
}

/// Returns true when an address family is running in a degraded state.
pub(crate) fn is_degraded(snapshot: &iface::Snapshot) -> bool {
    (cfg!(feature = "ipv4") && snapshot.v4_state == MethodState::Degraded)
        || (cfg!(feature = "ipv6") && snapshot.v6_state == MethodState::Degraded)
}

/// Returns true when at least one enabled family still exposes a method view.
pub(crate) const fn has_method_view(snapshot: &iface::Snapshot) -> bool {
    (cfg!(feature = "ipv4") && snapshot.v4.is_some())
        || (cfg!(feature = "ipv6") && snapshot.v6.is_some())
}

/// Returns true when the interface lost its carrier or a method gave up.
pub(crate) fn is_failed(snapshot: &iface::Snapshot) -> bool {
    !snapshot.link.has_carrier() || (is_degraded(snapshot) && !has_method_view(snapshot))
}
