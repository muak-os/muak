# Muak

Muak is a purpose built Linux distribution to run VMs using KVM/QEMU. It is designed to be minimal, immutable,
API-driven, secure, and easy to use.

Here are the two prerequisites to run Muak:

- Only run on UEFI systems
- System architecture is either `x86_64` or `arm64` hardware with virtualization support

## Build Process

Muak uses a custom build system to create a bootable ISO with a Unified Kernel Image (UKI):

1. **Kernel Build** (`scripts/build-kernel.sh`) - Compiles a minimal Linux kernel (6.15.11)
2. **Init Build** (`scripts/build-init.sh`) - Builds the Rust-based stage1 init system
3. **Initramfs Build** (`scripts/build-initramfs.sh`) - Creates initial ramdisk with:
   - Stage1 init (mounts filesystems, sets up loop devices, switches root)
   - SquashFS root filesystem containing stage2 init
4. **UKI Build** (`scripts/build-uki.sh`) - Assembles kernel, initramfs, and cmdline into UEFI image using ukify
5. **ISO Build** (`scripts/build-iso.sh`) - Creates bootable UEFI ISO image

## Boot Sequence

1. UEFI firmware loads UKI from ISO (`EFI/BOOT/BOOTX64.EFI`)
2. Linux kernel boots with embedded initramfs
3. Stage1 init (`/init`) runs:
   - Mounts pseudo filesystems (`/dev`, `/proc`, `/sys`, `/run`)
   - Attaches `/rootfs.sqsh` to loop device via ioctl
   - Mounts squashfs to `/newroot`
   - Moves pseudo filesystems to new root
   - Executes `switch_root` and transfers control to stage2
4. Stage2 init (`/sbin/init`) runs from squashfs rootfs

## TODO

- Create custom init to start api server and qemu vms
- Implement overlayfs for writable layer
- Add extension system (like Talos Linux)
- API to control VM lifecycle (using gRPC)
- SPICE server for remote graphical access to VMs
