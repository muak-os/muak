//! Hypervisor clock source (PTP device `/dev/ptp0`).

use core::time::Duration;

use anyhow::Result;
use rustix::fd::AsFd as _;
use rustix::fs::{Mode, OFlags, open};
use rustix::time::{DynamicClockId, Timespec, clock_gettime_dynamic};

use crate::clock;

/// Path of the first PTP hardware clock device registered by the kernel.
const PTP_DEVICE_PATH: &str = "/dev/ptp0";

/// Reads the current hypervisor clock time as a UNIX `Timespec`.
pub(crate) fn now() -> Result<Timespec> {
    let fd = open(PTP_DEVICE_PATH, OFlags::RDONLY, Mode::empty())?;
    let time = clock_gettime_dynamic(DynamicClockId::Dynamic(fd.as_fd()))?;

    Ok(time)
}

/// Sets the system clock from the hypervisor clock.
pub(crate) fn sync() -> Result<Duration> {
    Ok(clock::step(now()?)?)
}
