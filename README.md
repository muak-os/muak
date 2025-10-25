# Muak

Muak is a purpose built Linux distribution to run VMs using any hypervisors. It is designed to be minimal, immutable,
API-driven, secure, and easy to use.

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

   Examples:
   ```bash
   ./scripts/build-initramfs.sh x86_64                    # Base system (schematic: base)
   ./scripts/build-initramfs.sh x86_64 "firecracker"      # With firecracker
   ./scripts/build-initramfs.sh x86_64 "firecracker,qemu" # With multiple extensions
   ```

3. **UKI Build** (`scripts/build-uki.sh`)
   - Assembles kernel + initramfs + cmdline into UEFI executable using ukify

4. **ISO Build** (`scripts/build-iso.sh`)
   - Wraps UKI in bootable UEFI ISO image

## Extension System

Extensions are additional software packages (hypervisors, tools) layered on top of the base system:

- **Schematics**: Each extension combination gets a unique ID (SHA256 hash)
- **Build-time**: Extensions compiled to `.sqsh` files embedded in initramfs
- **Boot-time**: Stage1 init reads `/extensions.yaml` and mounts layers with overlayfs

## TODO

- Add SPICE extension server for remote graphical access to VMs
- Add gRPC authentication
- Add maintenance mode

qemu-system-x86_64 -enable-kvm -cpu host -m 2G -cdrom build/muak-x86_64.iso -bios /usr/share/ovmf/x64/OVMF.4m.fd -serial stdio -netdev user,id=net0,
hostfwd=tcp::50052-:50051 -device virtio-net-pci,netdev=net0
