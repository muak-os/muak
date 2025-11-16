use crate::log;
use crate::network::netlink::link;
use crate::network::services::bridge;
use anyhow::Result;
use rtnetlink::Handle;
use sha2::{Digest, Sha256};
use std::fs::OpenOptions;
use std::os::unix::io::AsRawFd;

const TUN_DEVICE: &str = "/dev/net/tun";
const IFF_TAP: i16 = 0x0002;
const IFF_NO_PI: i16 = 0x1000;
const IFF_VNET_HDR: i16 = 0x4000;

#[repr(C)]
struct IfReq {
    ifr_name: [u8; 16],
    ifr_flags: i16,
    _padding: [u8; 22],
}

nix::ioctl_write_ptr_bad!(tunsetiff, 0x400454ca, IfReq); // TUNSETIFF = 0x400454ca
nix::ioctl_write_int_bad!(tunsetpersist, 0x400454cb); // TUNSETPERSIST = 0x400454cb

pub async fn create_tap_device(tap_name: &str) -> Result<()> {
    log!("network", "Creating TAP device: {}", tap_name);

    let file = OpenOptions::new().read(true).write(true).open(TUN_DEVICE)?;

    let fd = file.as_raw_fd();

    let mut ifr = IfReq {
        ifr_name: [0u8; 16],
        ifr_flags: IFF_TAP | IFF_NO_PI | IFF_VNET_HDR,
        _padding: [0u8; 22],
    };

    let name_bytes = tap_name.as_bytes();
    let copy_len = name_bytes.len().min(15); // Leave room for null terminator
    ifr.ifr_name[..copy_len].copy_from_slice(&name_bytes[..copy_len]);

    unsafe { tunsetiff(fd, &ifr) }
        .map_err(|e| anyhow::anyhow!("failed to create TAP device: {}", e))?;

    unsafe { tunsetpersist(fd, 1) }
        .map_err(|e| anyhow::anyhow!("failed to make TAP device persistent: {}", e))?;

    log!("network", "Persistent TAP device {} created", tap_name);

    Ok(())
}

pub async fn setup_tap_on_bridge(
    handle: &Handle,
    tap_name: &str,
    bridge_name: &str,
) -> Result<u32> {
    create_tap_device(tap_name).await?;

    let index = link::ensure_link_up(handle, tap_name).await?;

    bridge::attach_to_bridge(handle, tap_name, bridge_name).await?;

    Ok(index)
}

pub async fn remove_tap_device(handle: &Handle, tap_name: &str) -> Result<()> {
    log!("network", "Deleting TAP device: {}", tap_name);

    if let Ok(index) = link::get_link_index(handle, tap_name).await {
        link::delete_link(handle, index).await?;
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
    // Bit 1 = locally administered, Bit 0 = unicast/multicast
    mac[0] = (mac[0] & 0xfe) | 0x02;

    mac
}

pub fn format_mac_address(mac: &[u8; 6]) -> String {
    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}
