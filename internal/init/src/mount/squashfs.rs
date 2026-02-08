//! Squashfs mounting via loop devices.

use std::os::fd::{AsFd, AsRawFd};

use anyhow::{Context, Result};
use rustix::fs::{Mode, OFlags, open};
use rustix::ioctl::{IntegerSetter, Opcode, ioctl};
use rustix::mount::{MountFlags, mount};

const LOOP_SET_FD: Opcode = 0x4C00;

/// Attach a squashfs image to a loop device and mount it.
pub fn attach_squashfs(sqsh_path: &str, loop_dev: &str, mount_point: &str) -> Result<()> {
    let sqsh_fd = open(sqsh_path, OFlags::RDONLY, Mode::empty())
        .with_context(|| format!("Failed to open squashfs image: {}", sqsh_path))?;
    let loop_fd = open(loop_dev, OFlags::RDWR, Mode::empty())
        .with_context(|| format!("Failed to open loop device: {}", loop_dev))?;

    let fd_number = sqsh_fd.as_fd().as_raw_fd() as usize;

    // SAFETY: ioctl is inherently unsafe, but IntegerSetter ensures proper argument passing
    unsafe {
        ioctl(&loop_fd, IntegerSetter::<LOOP_SET_FD>::new_usize(fd_number))
            .with_context(|| format!("Failed to attach {} to {}", sqsh_path, loop_dev))?;
    }

    mount(loop_dev, mount_point, "squashfs", MountFlags::RDONLY, None)
        .with_context(|| format!("Failed to mount {} to {}", loop_dev, mount_point))?;

    Ok(())
}
