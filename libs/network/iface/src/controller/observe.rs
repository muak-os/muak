//! Snapshot refresh and observation recording for method views.

use ip_method::Method;
#[cfg(feature = "ipv4")]
use net_observe::interface::DhcpLease;
use net_observe::interface::Family as ObserveFamily;
#[cfg(feature = "ipv6")]
use net_observe::interface::SlaacUpdate;
use net_observe::journal::Journal;
#[cfg(any(feature = "ipv4", feature = "ipv6"))]
use net_observe::{Event as ObserveEvent, Level};

#[cfg(feature = "ipv6")]
use crate::V6View;
use crate::{Controller, Family, V4View};

pub(super) fn refresh(controller: &mut Controller, family: Family, journal: &mut Journal) {
    match family {
        Family::V4 => {
            let view = controller.v4.as_ref().and_then(Method::view);
            let changed = !same_v4(controller.snapshot.v4.as_ref(), view.as_ref());
            controller.snapshot.v4 = view;
            if changed {
                record_v4(controller, journal);
            }
        }
        Family::V6 => {
            let view = controller.v6.as_ref().and_then(Method::view);
            let changed = view != controller.snapshot.v6;
            controller.snapshot.v6 = view;
            if changed {
                record_v6(controller, journal);
            }
        }
    }
}

pub(super) fn same_v4(old: Option<&V4View>, new: Option<&V4View>) -> bool {
    match (old, new) {
        (None, None) => true,
        (Some(old), Some(new)) => same_v4_views(old, new),
        _ => false,
    }
}

#[cfg(feature = "ipv4")]
pub(super) fn record_v4(controller: &Controller, journal: &mut Journal) {
    let Some(view) = controller.snapshot.v4.as_ref() else {
        return;
    };
    let V4View::Dhcp(ref view) = *view else {
        return;
    };
    let lease = DhcpLease {
        address: view.lease.assigned_ip,
        prefix_len: view.lease.prefix_len,
        server: view.lease.server_ip,
        gateway: view.lease.gateway,
        dns: view.lease.dns_servers.clone(),
        phase: dhcp_phase(view.phase),
    };
    let level = if view.phase == dhcp::Phase::Degraded {
        Level::Warn
    } else {
        Level::Info
    };

    journal.record(
        level,
        ObserveEvent::DhcpLease {
            name: controller.link.name.clone(),
            lease,
        },
    );
}

#[cfg(not(feature = "ipv4"))]
const fn record_v4(_controller: &Controller, _journal: &mut Journal) {}

pub(super) const fn family(family: Family) -> ObserveFamily {
    match family {
        Family::V4 => ObserveFamily::V4,
        Family::V6 => ObserveFamily::V6,
    }
}

fn same_v4_views(old: &V4View, new: &V4View) -> bool {
    match *old {
        V4View::Static(ref old) => match *new {
            V4View::Static(ref new) => old == new,
            #[cfg(feature = "ipv4")]
            V4View::Dhcp(_) => false,
        },
        #[cfg(feature = "ipv4")]
        V4View::Dhcp(ref old) => match *new {
            V4View::Static(_) => false,
            V4View::Dhcp(ref new) => {
                old.phase == new.phase
                    && old.lease.assigned_ip == new.lease.assigned_ip
                    && old.lease.prefix_len == new.lease.prefix_len
                    && old.lease.server_ip == new.lease.server_ip
                    && old.lease.gateway == new.lease.gateway
                    && old.lease.dns_servers == new.lease.dns_servers
            }
        },
    }
}

#[cfg(feature = "ipv6")]
fn record_v6(controller: &Controller, journal: &mut Journal) {
    let Some(view) = controller.snapshot.v6.as_ref() else {
        return;
    };
    let V6View::Slaac(ref view) = *view else {
        return;
    };
    let update = SlaacUpdate {
        addresses: view.addresses.clone(),
        routers: view.routers.clone(),
        dns: view.dns.clone(),
    };

    journal.record(
        Level::Info,
        ObserveEvent::SlaacUpdate {
            name: controller.link.name.clone(),
            update,
        },
    );
}

#[cfg(not(feature = "ipv6"))]
const fn record_v6(_controller: &Controller, _journal: &mut Journal) {}

#[cfg(feature = "ipv4")]
const fn dhcp_phase(phase: dhcp::Phase) -> net_observe::interface::DhcpPhase {
    match phase {
        dhcp::Phase::Idle => net_observe::interface::DhcpPhase::Idle,
        dhcp::Phase::Acquiring => net_observe::interface::DhcpPhase::Acquiring,
        dhcp::Phase::Bound => net_observe::interface::DhcpPhase::Bound,
        dhcp::Phase::Renewing => net_observe::interface::DhcpPhase::Renewing,
        dhcp::Phase::Rebinding => net_observe::interface::DhcpPhase::Rebinding,
        dhcp::Phase::Degraded => net_observe::interface::DhcpPhase::Degraded,
        dhcp::Phase::Stopped => net_observe::interface::DhcpPhase::Stopped,
    }
}
