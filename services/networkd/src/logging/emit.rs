//! Kernel log emission of a rendered observation.

use net_observe::{Level, Observation};

use super::Line;

/// Emits one journaled observation to the kernel log at its recorded level.
pub(crate) fn emit(observation: Observation) {
    let Observation { level, event } = observation;
    let line = Line(&event);

    match level {
        Level::Debug => {
            kmsg::debug!("{line}");
        }
        Level::Info => {
            kmsg::info!("{line}");
        }
        Level::Warn => {
            kmsg::warn!("{line}");
        }
        Level::Error => {
            kmsg::error!("{line}");
        }
    }
}
