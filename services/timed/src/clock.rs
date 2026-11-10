//! Step of the realtime clock to an externally supplied target time.

use core::time::Duration;

use rustix::io::Errno;
use rustix::time::{ClockId, Timespec, clock_gettime, clock_settime};

/// Sets the realtime clock to `target` and returns the whole-second offset
/// between the previous and the new clock time.
pub(crate) fn step(target: Timespec) -> Result<Duration, Errno> {
    let before = clock_gettime(ClockId::Realtime);

    clock_settime(ClockId::Realtime, target)?;

    let offset_secs = target.tv_sec.abs_diff(before.tv_sec);

    Ok(Duration::from_secs(offset_secs))
}
