use crate::log;
use futures::stream::TryStreamExt;
use rtnetlink::Handle;
use sha2::{Digest, Sha256};
use std::fs::OpenOptions;
use std::os::unix::io::AsRawFd;

const TUN_DEVICE: &str = "/dev/net/tun";
const TUNSETIFF: u64 = 0x400454ca;
const TUNSETPERSIST: u64 = 0x400454cb;
const IFF_TAP: i16 = 0x0002;
const IFF_NO_PI: i16 = 0x1000;
const IFF_VNET_HDR: i16 = 0x4000;

#[repr(C)]
struct IfReq {
    ifr_name: [u8; 16],
    ifr_flags: i16,
    _padding: [u8; 22],
}

pub async fn create_tap(tap_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    log!("network", "Creating TAP device: {}", tap_name);

    let file = OpenOptions::new().read(true).write(true).open(TUN_DEVICE)?;

    let fd = file.as_raw_fd();

    let mut ifr = IfReq {
        ifr_name: [0u8; 16],
        ifr_flags: IFF_TAP | IFF_NO_PI | IFF_VNET_HDR,
        _padding: [0u8; 22],
    };

    let name_bytes = tap_name.as_bytes();
    let copy_len = name_bytes.len().min(15);
    ifr.ifr_name[..copy_len].copy_from_slice(&name_bytes[..copy_len]);

    let ret = unsafe { nix::libc::ioctl(fd, TUNSETIFF, &ifr as *const IfReq) };

    if ret < 0 {
        return Err("Failed to create TAP device".into());
    }

    let persist_ret = unsafe { nix::libc::ioctl(fd, TUNSETPERSIST, 1) };

    if persist_ret < 0 {
        return Err("Failed to make TAP device persistent".into());
    }

    log!("network", "Persistent TAP device {} created", tap_name);

    Ok(())
}

pub async fn bring_up_tap(
    handle: &Handle,
    tap_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    log!("network", "Bringing up TAP device: {}", tap_name);

    let mut links = handle
        .link()
        .get()
        .match_name(tap_name.to_string())
        .execute();

    if let Some(link) = links.try_next().await? {
        handle.link().set(link.header.index).up().execute().await?;
        log!("network", "TAP device {} is up", tap_name);
    } else {
        return Err(format!("TAP device {} not found", tap_name).into());
    }

    Ok(())
}

pub async fn delete_tap(handle: &Handle, tap_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    log!("network", "Deleting TAP device: {}", tap_name);

    let mut links = handle
        .link()
        .get()
        .match_name(tap_name.to_string())
        .execute();

    if let Some(link) = links.try_next().await? {
        handle.link().del(link.header.index).execute().await?;
        log!("network", "TAP device {} deleted", tap_name);
    } else {
        log!("network", "TAP device {} does not exist", tap_name);
    }

    Ok(())
}

pub fn generate_mac_address(vm_id: &str) -> [u8; 6] {
    let mut hasher = Sha256::new();
    hasher.update(vm_id.as_bytes());
    let result = hasher.finalize();

    let mut mac = [0u8; 6];
    mac.copy_from_slice(&result[0..6]);

    // Set the locally administered bit and clear the multicast bit
    mac[0] = (mac[0] & 0xfe) | 0x02;

    mac
}

pub fn format_mac_address(mac: &[u8; 6]) -> String {
    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}
