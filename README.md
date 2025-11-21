# Muak

Muak is a purpose built from scratch Linux distribution to run VMs using hypervisors such as QEMU, firecracker and cloud-hypervisor.
It is designed to be minimal, immutable, API-driven, secure, and easy to use.

Here are the three prerequisites to run Muak:

- Only runs on UEFI systems
- System architecture is either `x86_64` or `arm64` hardware with virtualization support enabled in firmware
- A full disk reserved for Muak installation

Here is a list of features:

- Immutable root filesystem with overlayfs for persistence
- Declarative config by design
- API driven using gRPC
- Only one external binary by default: `btrfs-progs`
