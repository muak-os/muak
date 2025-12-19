// Only MUAK_DISKS_DIR is currently used by main.rs
// Other constants are kept for when grpcd/vmd are extracted
#![allow(dead_code)]

pub const CLOUD_HYPERVISOR_BINARY: &str = "/usr/bin/cloud-hypervisor";
pub const FIRECRACKER_BINARY: &str = "/usr/bin/firecracker";
pub const FIRECRACKER_KERNEL_PATH: &str = "/usr/share/firecracker/vmlinux";
pub const FIRECRACKER_ROOTFS_PATH: &str = "/usr/share/firecracker/rootfs.ext4";
pub const UEFI_FIRMWARE_PATH: &str = "/usr/share/muak/CLOUDHV.fd";
pub const GRANOLA_SOCKET_PATH: &str = "/run/granola.sock";
pub const MUAK_DISKS_DIR: &str = "/run/state/images";
pub const GRPC_SERVER_ADDR: &str = "0.0.0.0:50051";
pub const IPC_MAX_MESSAGE_SIZE: usize = 10 * 1024 * 1024;
