//! Periodic reconciliation of the declared network intent.

use std::time::Duration;

use crate::{Supervisor, dns, provision};

/// Interval between two reconciliation passes.
pub(crate) const RECONCILE_INTERVAL: Duration = Duration::from_secs(30);

/// Reapplies every configured interface and refreshes DNS while operational.
pub(crate) fn run(supervisor: &mut Supervisor) {
    if !supervisor.state.is_operational() {
        return;
    }

    for index in 0..supervisor.config.interfaces.len() {
        provision::reconcile_interface(supervisor, index);
    }
    dns::flush(supervisor);
}
