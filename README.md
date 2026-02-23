# Muak

Muak is a Linux distribution built from scratch to run VMs using hypervisors such as QEMU, firecracker and cloud-hypervisor.
It is designed to be minimal, immutable, API-driven, atomic, secure and easy to use.

Here is a list of features:

- Immutable root filesystem with overlayfs to add extensions
- Declarative config by design to prevent configuration drift
- API driven using gRPC with mTLS authentication
- Minimal with no external binaries except your hypervisor of choice

There are three prerequisites to run Muak that most if not all modern systems meet:

- System architecture is either `x86_64` or `arm64` hardware with virtualization support enabled in firmware
- A full disk reserved for the installation
- UEFI firmware
