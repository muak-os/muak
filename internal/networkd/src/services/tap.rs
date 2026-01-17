use crate::netlink::link;
use crate::services::bridge;
use anyhow::{Context, Result};
use rtnetlink::Handle;
use rustix::fs::{Mode, OFlags, open};
use rustix::ioctl::{Ioctl, IoctlOutput, Opcode, ioctl};
use sha2::{Digest, Sha256};

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

struct TunSetIffIoctl<'a> {
    ifreq: &'a IfReq,
}

impl<'a> TunSetIffIoctl<'a> {
    fn new(ifreq: &'a IfReq) -> Self {
        Self { ifreq }
    }
}

unsafe impl Ioctl for TunSetIffIoctl<'_> {
    type Output = ();

    const IS_MUTATING: bool = true;

    // TUNSETIFF = _IOW('T', 202, int)
    fn opcode(&self) -> Opcode {
        0x400454ca
    }

    fn as_ptr(&mut self) -> *mut std::ffi::c_void {
        self.ifreq as *const IfReq as *mut std::ffi::c_void
    }

    unsafe fn output_from_ptr(
        _output: IoctlOutput,
        _arg: *mut std::ffi::c_void,
    ) -> rustix::io::Result<Self::Output> {
        Ok(())
    }
}

struct TunSetPersistIoctl {
    value: i32,
}

impl TunSetPersistIoctl {
    fn new(value: i32) -> Self {
        Self { value }
    }
}

unsafe impl Ioctl for TunSetPersistIoctl {
    type Output = ();

    const IS_MUTATING: bool = true;

    // TUNSETPERSIST = _IOW('T', 203, int)
    fn opcode(&self) -> Opcode {
        0x400454cb
    }

    fn as_ptr(&mut self) -> *mut std::ffi::c_void {
        self.value as usize as *mut std::ffi::c_void
    }

    unsafe fn output_from_ptr(
        _output: IoctlOutput,
        _arg: *mut std::ffi::c_void,
    ) -> rustix::io::Result<Self::Output> {
        Ok(())
    }
}

pub async fn create_tap_device(tap_name: &str) -> Result<()> {
    kmsg::info!(@ "networkd", "Creating TAP device: {}", tap_name);

    let file =
        open(TUN_DEVICE, OFlags::RDWR, Mode::empty()).context("Failed to open tun device")?;

    let ifr = IfReq {
        ifr_name: {
            let mut name = [0u8; 16];
            let name_bytes = tap_name.as_bytes();
            let copy_len = name_bytes.len().min(15); // Leave room for null terminator
            name[..copy_len].copy_from_slice(&name_bytes[..copy_len]);
            name
        },
        ifr_flags: IFF_TAP | IFF_NO_PI | IFF_VNET_HDR,
        _padding: [0u8; 22],
    };

    unsafe { ioctl(&file, TunSetIffIoctl::new(&ifr)) }
        .map_err(|e| anyhow::anyhow!("failed to create TAP device: {}", e))?;

    unsafe { ioctl(&file, TunSetPersistIoctl::new(1)) }
        .map_err(|e| anyhow::anyhow!("failed to make TAP device persistent: {}", e))?;

    kmsg::info!(@ "networkd", "Persistent TAP device {} created", tap_name);

    Ok(())
}

pub async fn setup_tap_on_bridge(
    handle: &Handle,
    tap_name: &str,
    bridge_name: &str,
) -> Result<u32> {
    create_tap_device(tap_name).await?;

    let link = link::find_link_by_name(handle, tap_name).await?;
    let index = link.header.index;

    link::bring_link_up(handle, index).await?;

    bridge::attach_to_bridge(handle, tap_name, bridge_name).await?;

    Ok(index)
}

pub async fn remove_tap_device(handle: &Handle, tap_name: &str) -> Result<()> {
    kmsg::info!(@ "networkd", "Deleting TAP device: {}", tap_name);

    if let Ok(index) = link::get_link_index(handle, tap_name).await {
        link::delete_link(handle, index).await?;
        kmsg::info!(@ "networkd", "TAP device {} deleted", tap_name);
    } else {
        kmsg::info!(@ "networkd", "TAP device {} does not exist", tap_name);
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
