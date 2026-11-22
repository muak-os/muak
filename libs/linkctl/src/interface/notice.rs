//! Transition notice helpers for interface controllers.

use alloc::string::String;

use netdev::Ops;

use super::Interface;
#[cfg(feature = "ipv4")]
use super::V4Notice;
#[cfg(feature = "ipv6")]
use super::V6Notice;
#[cfg(feature = "ipv4")]
use super::v4;
#[cfg(feature = "ipv6")]
use super::v6;
use crate::provision;

#[cfg(feature = "ipv4")]
pub(super) fn notify_v4<N: Ops>(interface: &Interface<N>, notice: V4Notice) {
    drop(interface.v4_notice_tx.try_send(notice));
}

#[cfg(feature = "ipv6")]
pub(super) fn notify_v6<N: Ops>(interface: &Interface<N>, notice: V6Notice) {
    drop(interface.v6_notice_tx.try_send(notice));
}

#[cfg(feature = "ipv4")]
pub(super) fn transition_v4<N: Ops>(interface: &mut Interface<N>, state: provision::State) {
    if interface.v4_state != state {
        interface.v4_state = state;
        notify_v4(interface, V4Notice::State(state));
    }
    if state == provision::State::Bound {
        let view = interface.v4.as_ref().and_then(v4::Method::view);
        if let (false, Some(view)) = (interface.v4_configured, view) {
            notify_v4(interface, V4Notice::Configured(view));
            interface.v4_configured = true;
        }
    }
    if matches!(
        state,
        provision::State::Acquiring | provision::State::Stopped
    ) {
        interface.v4_configured = false;
    }
}

#[cfg(feature = "ipv6")]
pub(super) fn transition_v6<N: Ops>(interface: &mut Interface<N>, state: provision::State) {
    if interface.v6_state != state {
        interface.v6_state = state;
        notify_v6(interface, V6Notice::State(state));
    }
    if state == provision::State::Bound {
        let view = interface.v6.as_ref().and_then(v6::Method::view);
        if let (false, Some(view)) = (interface.v6_configured, view) {
            notify_v6(interface, V6Notice::Configured(view));
            interface.v6_configured = true;
        }
    }
    if matches!(
        state,
        provision::State::Acquiring | provision::State::Stopped
    ) {
        interface.v6_configured = false;
    }
}

#[cfg(feature = "ipv4")]
pub(super) fn fail_v4<N: Ops>(interface: &mut Interface<N>, warning: String) {
    notify_v4(interface, V4Notice::Warning(warning));
    transition_v4(interface, provision::State::Degraded);
}

#[cfg(feature = "ipv6")]
pub(super) fn fail_v6<N: Ops>(interface: &mut Interface<N>, warning: String) {
    notify_v6(interface, V6Notice::Warning(warning));
    transition_v6(interface, provision::State::Degraded);
}
