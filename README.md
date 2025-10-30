# Muak

Muak is a purpose built Linux distribution to run VMs using hypervisors such as QEMU, firecracker and cloud-hypervisor.
It is designed to be minimal, immutable, API-driven, secure, and easy to use.

Here are the two prerequisites to run Muak:

- Only runs on UEFI systems
- System architecture is either `x86_64` or `arm64` hardware with virtualization support enabled in firmware

## Build Process

Muak uses a custom build system to create a bootable ISO with a Unified Kernel Image (UKI):

1. **Kernel Build** (`scripts/build-kernel.sh`)
   - Compiles minimal Linux kernel (6.15.11) with overlayfs support

2. **Initramfs Build** (`scripts/build-initramfs.sh [arch] [extensions]`)
   - Builds stage1 init (statically linked Rust binary)
   - Creates base rootfs squashfs (granola PID 1)
   - Builds extension squashfs files (if specified)
   - Generates `extensions.yaml` manifest
   - Creates schematic ID (deterministic hash of extensions)
   - Packages everything into `build/initramfs.img`

3. **UKI Build** (`scripts/build-uki.sh`)
   - Assembles kernel + initramfs + cmdline into UEFI executable using ukify

4. **ISO Build** (`scripts/build-iso.sh`)
   - Wraps UKI in bootable UEFI ISO image

## Extension System

Extensions are additional software packages (hypervisors, tools) layered on top of the base system:

- **Schematics**: Each extension combination gets a unique ID (SHA256 hash)
- **Build-time**: Extensions compiled to `.sqsh` files embedded in initramfs
- **Boot-time**: Stage1 init reads `/extensions.yaml` and mounts layers with overlayfs

