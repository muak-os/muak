# Muak

Muak is a Linux distribution built from scratch to run VMs using hypervisors such as QEMU, firecracker and cloud-hypervisor.
It is designed to be minimal, immutable, API-driven, atomic, secure and easy to use.

Here are the three prerequisites to run Muak that most if not all modern systems meet:

- Only runs on UEFI systems
- System architecture is either `x86_64` or `arm64` hardware with virtualization support enabled in firmware
- A full disk reserved for Muak installation

Here is a list of features:

- Immutable root filesystem with overlayfs to add extensions
- Declarative config by design
- API driven using gRPC
- Only one external binary by default: `btrfs-progs`

## Development

### Prerequisites

You need the following tools installed on your host system: `git`, `musl`, `rustup`, `make` and `docker` or `podman`

Add the following targets:

```sh
rustup target add x86_64-unknown-linux-musl
rustup target add aarch64-unknown-linux-musl
rustup target add x86_64-unknown-uefi --toolchain nightly
rustup target add aarch64-unknown-uefi --toolchain nightly
```

### Quick Start

```sh
make kernel
make dev
qemu-system-x86_64 -enable-kvm -cpu host -m 2G \
    -cdrom _out/muak-x86_64.iso \
    -bios /usr/share/ovmf/x64/OVMF.4m.fd \
    -serial stdio
```

### Build locally with extensions

```sh
make kernel
make local-cloud-hypervisor REGISTRY=localhost
make dev EXTENSIONS="_out/oci/cloud-hypervisor"
```
