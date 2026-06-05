//! TAP device life cycle.

use rtnetlink::Handle;
use rustix::fs::{Mode, OFlags, open};
use rustix::io::Errno;
use rustix::ioctl::{Opcode, Setter, ioctl};
use thiserror::Error;

use crate::link;

const TUN_DEVICE: &str = "/dev/net/tun";
const IFF_TAP: i16 = 0x0002;
const IFF_NO_PI: i16 = 0x1000;
const IFF_VNET_HDR: i16 = 0x4000;

const TUNSETIFF: Opcode = 0x4004_54ca;
const TUNSETPERSIST: Opcode = 0x4004_54cb;

#[repr(C)]
struct IfReq {
    ifr_name: [u8; 16],
    ifr_flags: i16,
    _padding: [u8; 22],
}

/// TAP device operation failures.
#[derive(Debug, Error)]
pub enum Failure {
    /// Failed to open tun device.
    #[error("failed to open tun device: {0}")]
    OpenTun(#[source] Errno),
    /// Failed to create TAP device.
    #[error("failed to create TAP device: {0}")]
    IoctlSetIff(#[source] Errno),
    /// Failed to make TAP device persistent.
    #[error("failed to make TAP device persistent: {0}")]
    IoctlSetPersist(#[source] Errno),
    /// Link operation error.
    #[error(transparent)]
    Link(#[from] link::Failure),
}

/// TAP device operation result type.
pub type Result<T> = core::result::Result<T, Failure>;

fn build_ifreq_name(tap_name: &str) -> [u8; 16] {
    let mut name = [0_u8; 16];
    let name_bytes = tap_name.as_bytes();
    let copy_len = name_bytes.len().min(15);

    if let (Some(dst), Some(src)) = (name.get_mut(..copy_len), name_bytes.get(..copy_len)) {
        dst.copy_from_slice(src);
    }

    name
}

/// Creates a persistent TAP device via /dev/net/tun with virtio headers enabled.
///
/// # Errors
///
/// Returns an error when the tun device cannot be opened or the required ioctls fail.
pub fn create(tap_name: &str) -> Result<()> {
    println!("Creating TAP device: {tap_name}");

    let file = open(TUN_DEVICE, OFlags::RDWR, Mode::empty()).map_err(Failure::OpenTun)?;

    let ifr = IfReq {
        ifr_name: build_ifreq_name(tap_name),
        ifr_flags: IFF_TAP | IFF_NO_PI | IFF_VNET_HDR,
        _padding: [0_u8; 22],
    };

    let set_iff = unsafe {
        // SAFETY: `Setter::new` packages a plain old data `ifreq` for `TUNSETIFF`.
        Setter::<TUNSETIFF, IfReq>::new(ifr)
    };
    unsafe {
        // SAFETY: The setter opcode and argument layout match the `TUNSETIFF` ioctl contract.
        ioctl(&file, set_iff)
    }
    .map_err(Failure::IoctlSetIff)?;

    let set_persist = unsafe {
        // SAFETY: `Setter::new` packages the persistence flag for `TUNSETPERSIST`.
        Setter::<TUNSETPERSIST, i32>::new(1)
    };
    unsafe {
        // SAFETY: The setter opcode and argument layout match the `TUNSETPERSIST` ioctl contract.
        ioctl(&file, set_persist)
    }
    .map_err(Failure::IoctlSetPersist)?;

    println!("Persistent TAP device {tap_name} created");

    Ok(())
}

/// Creates a TAP device, brings it up, and attaches it to the named bridge.
///
/// # Errors
///
/// Returns an error when the TAP device cannot be created, brought up, or attached to the bridge.
pub async fn setup_on_bridge(handle: &Handle, tap_name: &str, bridge_name: &str) -> Result<u32> {
    create(tap_name)?;

    let msg = link::find_by_name(handle, tap_name).await?;
    let index = msg.header.index;

    link::bring_up(handle, index).await?;

    let bridge_index = link::get_index(handle, bridge_name).await?;
    link::set_master(handle, index, bridge_index).await?;
    println!("{tap_name} attached to bridge {bridge_name}");

    Ok(index)
}

/// Deletes a TAP device by name (no-op if the device does not exist).
///
/// # Errors
///
/// Returns an error when deleting an existing TAP device fails.
pub async fn remove(handle: &Handle, tap_name: &str) -> Result<()> {
    println!("Deleting TAP device: {tap_name}");

    if let Ok(index) = link::get_index(handle, tap_name).await {
        link::delete(handle, index).await?;
        println!("TAP device {tap_name} deleted");
    } else {
        println!("TAP device {tap_name} does not exist");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_ifreq_name_truncates_to_kernel_limit() {
        // ARRANGE
        let name = "tap-name-that-is-too-long";

        // ACT
        let ifreq_name = build_ifreq_name(name);

        // ASSERT
        assert_eq!(&ifreq_name[..15], b"tap-name-that-i");
        assert_eq!(ifreq_name[15], 0);
    }

    #[test]
    fn build_ifreq_name_zero_fills_unused_tail() {
        // ARRANGE
        let name = "tap0";

        // ACT
        let ifreq_name = build_ifreq_name(name);

        // ASSERT
        assert_eq!(&ifreq_name[..4], b"tap0");
        assert!(ifreq_name[4..].iter().all(|byte| *byte == 0));
    }
}
