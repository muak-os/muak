# Architecture

Muak is designed to be **atomic** in deployment and **modular** in composition.

It is atomic in that the entirety of Muak is distributed as a single, self-contained image — a Unified Kernel Image (UKI) containing the UEFI stub, kernel, and initramfs as a single signed PE binary. Every node running the same image is byte-for-byte identical.

It is modular in that it is composed of a small set of purpose-built daemons with clearly defined gRPC interfaces. All inter-daemon communication happens over Unix domain sockets using the same gRPC protocol exposed externally. This imposes a clear separation of concerns and ensures that changes affecting component interoperation are part of the public API contract. Each component can evolve independently as long as its interface is controlled.

## Disk Partitions

Muak uses a fixed three-partition layout on a single disk:

1. **EFI** — FAT32 partition (512 MB) storing the UKI binary (`BOOTX64.EFI` or `BOOTAA64.EFI`).
2. **STATE** — LUKS2-encrypted Btrfs partition (1 GB) storing machine configuration, secrets, update staging, config history, and VM state metadata.
3. **DATA** — LUKS2-encrypted Btrfs partition (remainder of disk) storing VM disk subvolumes and persistent disk images.

The minimum supported disk size is 2 GB. The target disk is selected at install time via `system.disk` in the machine configuration and cannot be changed afterward.

## The File System

Muak's root file system is the initramfs itself — a gzip-compressed CPIO archive loaded entirely into memory at boot. It is never written to disk; every node boots from the same in-memory image. This provides an immutable base: the running OS cannot be modified, and there is no configuration drift.

The initramfs contains all Muak binaries statically linked against musl libc. There are no shared libraries, no dynamic linker, and no package manager. The only writable state on the host lives on the two encrypted partitions:

- `/run/state` — mounted from the STATE partition (LUKS2/Btrfs). Holds `config.toml`, `auth.toml`, TLS secrets, and operational state.
- `/run/data` — mounted from the DATA partition (LUKS2/Btrfs). Holds VM disk subvolumes and persistent disk images.

All files under `/run/state` and `/run/data` persist across reboots and updates. Everything else — logs, runtime sockets, temporary files — lives in `tmpfs` and is recreated on each boot.
