//! Hypervisor clock source (PTP device `/dev/ptp0`).

use core::time::Duration;

use anyhow::Result;
use rustix::fd::AsFd as _;
use rustix::fs::{Mode, OFlags, open};
use rustix::time::{
    ClockId, DynamicClockId, Timespec, clock_gettime, clock_gettime_dynamic, clock_settime,
};

/// Path of the first PTP hardware clock device registered by the kernel.
const PTP_DEVICE_PATH: &str = "/dev/ptp0";

/// Reads the current hypervisor clock time as a UNIX `Timespec`.
pub fn now() -> Result<Timespec> {
    let fd = open(PTP_DEVICE_PATH, OFlags::RDONLY, Mode::empty())?;
    let time = clock_gettime_dynamic(DynamicClockId::Dynamic(fd.as_fd()))?;

    Ok(time)
}

/// Sets the system clock from the hypervisor clock.
pub fn sync() -> Result<Duration> {
    let host_time = now()?;

    let before = clock_gettime(ClockId::Realtime);

    clock_settime(ClockId::Realtime, host_time)?;

    let offset_secs = host_time.tv_sec.abs_diff(before.tv_sec);

    Ok(Duration::from_secs(offset_secs))
}
