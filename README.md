# Muak

Muak is a Linux distribution built from scratch to run Virtual Machines using hypervisors such as QEMU, Firecracker and cloud-hypervisor. It is created from the ground up for that sole purpose, ensuring maximum performance, stability, and ease of use for virtualization.

It is the most lightweight Linux distribution you'll probably use while still being fully functional for running VMs.

## Features

- **Immutable root filesystem** with overlayfs to add extensions
- **Declarative configuration by design** to prevent configuration drift
- **API driven** using gRPC with mTLS authentication
- **Atomic updates** for safe, predictable updates/rollbacks
- **Minimal** with no external binaries except your hypervisor of choice
- **Secure by default**

## Requirements

There are three prerequisites to run Muak that most modern systems meet:

- System architecture is either `x86_64` or `arm64` hardware with virtualization support enabled in firmware
- UEFI firmware
- A full disk reserved for the installation

Muak can be deployed anywhere you can run a modern Linux distribution.

## Why Muak?

Muak has a number of features that make it ideal for virtualization:

### API Managed

Muak is managed by a gRPC API instead of a traditional shell, much like other distributions such as [Talos](https://talos.dev).

### Immutable Filesystem

Muak uses an immutable filesystem to ensure that the base system remains unchanged and secure. This means that any changes made to the system are ephemeral and will be lost upon reboot, ensuring a clean state every time. Only a small state-persistent partition is used to store the data needed to make Muak functional.

### No External Binaries

Muak does not include any external binaries by default, except for the hypervisor you choose to run (e.g., cloud-hypervisor, QEMU). This minimizes the attack surface and ensures that only necessary components are present on the system.
