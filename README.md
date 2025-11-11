# Muak

Muak is a purpose built from scratch Linux distribution to run VMs using hypervisors such as QEMU, firecracker and cloud-hypervisor.
It is designed to be minimal, immutable, API-driven, secure, and easy to use.

Here are the two prerequisites to run Muak:

- Only runs on UEFI systems
- System architecture is either `x86_64` or `arm64` hardware with virtualization support enabled in firmware

Here is a list of features:

- Only one external binaries by default: `btrfs-progs`
- Immutable root filesystem with overlayfs for persistence
- API driven using gRPC
- Systemd free

## Extension System

Extensions are additional software packages (hypervisors, tools) layered on top of the base system:

- **Schematics**: Each extension combination gets a unique ID (SHA256 hash)
- **Build-time**: Extensions compiled to `.sqsh` files embedded in initramfs
- **Boot-time**: Stage1 init reads `/extensions.yaml` and mounts layers with overlayfs

