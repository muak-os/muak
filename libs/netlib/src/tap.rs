//! TAP device life cycle.

use rtnetlink::Handle;
use rustix::fs::{Mode, OFlags, open};
use rustix::ioctl::{Opcode, Setter, ioctl};
use thiserror::Error;

use crate::link;

const TUN_DEVICE: &str = "/dev/net/tun";
const IFF_TAP: i16 = 0x0002;
const IFF_NO_PI: i16 = 0x1000;
const IFF_VNET_HDR: i16 = 0x4000;

const TUNSETIFF: Opcode = 0x400454ca;
const TUNSETPERSIST: Opcode = 0x400454cb;

#[repr(C)]
struct IfReq {
    ifr_name: [u8; 16],
    ifr_flags: i16,
    _padding: [u8; 22],
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to open tun device: {0}")]
    OpenTun(#[source] rustix::io::Errno),
    #[error("failed to create TAP device: {0}")]
    IoctlSetIff(#[source] rustix::io::Errno),
    #[error("failed to make TAP device persistent: {0}")]
    IoctlSetPersist(#[source] rustix::io::Errno),
    #[error(transparent)]
    Link(#[from] link::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Creates a persistent TAP device via /dev/net/tun with virtio headers enabled.
pub async fn create(tap_name: &str) -> Result<()> {
    println!("Creating TAP device: {}", tap_name);

    let file = open(TUN_DEVICE, OFlags::RDWR, Mode::empty()).map_err(Error::OpenTun)?;

    let ifr = IfReq {
        ifr_name: {
            let mut name = [0u8; 16];
            let name_bytes = tap_name.as_bytes();
            let copy_len = name_bytes.len().min(15);
            name[..copy_len].copy_from_slice(&name_bytes[..copy_len]);
            name
        },
        ifr_flags: IFF_TAP | IFF_NO_PI | IFF_VNET_HDR,
        _padding: [0u8; 22],
    };

    // SAFETY: ioctl is inherently unsafe, but Setter ensures proper argument passing
    unsafe { ioctl(&file, Setter::<TUNSETIFF, IfReq>::new(ifr)) }.map_err(Error::IoctlSetIff)?;

    // SAFETY: ioctl is inherently unsafe, but Setter ensures proper argument passing
    unsafe { ioctl(&file, Setter::<TUNSETPERSIST, i32>::new(1)) }
        .map_err(Error::IoctlSetPersist)?;

    println!("Persistent TAP device {} created", tap_name);

    Ok(())
}

/// Creates a TAP device, brings it up, and attaches it to the named bridge.
pub async fn setup_on_bridge(handle: &Handle, tap_name: &str, bridge_name: &str) -> Result<u32> {
    create(tap_name).await?;

    let msg = link::find_by_name(handle, tap_name).await?;
    let index = msg.header.index;

    link::bring_up(handle, index).await?;

    let bridge_index = link::get_index(handle, bridge_name).await?;
    link::set_master(handle, index, bridge_index).await?;
    println!("{} attached to bridge {}", tap_name, bridge_name);

    Ok(index)
}

/// Deletes a TAP device by name (no-op if the device does not exist).
pub async fn remove(handle: &Handle, tap_name: &str) -> Result<()> {
    println!("Deleting TAP device: {}", tap_name);

    if let Ok(index) = link::get_index(handle, tap_name).await {
        link::delete(handle, index).await?;
        println!("TAP device {} deleted", tap_name);
    } else {
        println!("TAP device {} does not exist", tap_name);
    }

    Ok(())
}
