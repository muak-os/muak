# Muak

Muak is a Linux distribution built from scratch to run VMs using hypervisors such as QEMU, firecracker and cloud-hypervisor.
It is designed to be minimal, immutable, API-driven, atomic, secure and easy to use.

Here is a list of features:

- Immutable root filesystem with overlayfs to add extensions
- Declarative config by design to prevent configuration drift
- API driven using gRPC with mTLS authentication
- Minimal with only one external binary by default

There are three prerequisites to run Muak that most if not all modern systems meet:

- System architecture is either `x86_64` or `arm64` hardware with virtualization support enabled in firmware
- A full disk reserved for the installation
- UEFI firmware

## Development

### Prerequisites

You need the following tools installed on your host system: `git`, `musl`, `rustup`, `just` and `docker` or `podman`

Add the following targets:

```sh
rustup target add x86_64-unknown-linux-musl
rustup component add rust-analyzer
rustup target add x86_64-unknown-uefi --toolchain nightly
rustup component add rust-analyzer --toolchain nightly
```

### Quick Start

```sh
# Create a signing key for the kernel and place it in pkgs/kernel/
SIGNING_ARGS="--secret id=kernel_key,src=pkgs/kernel/kernel-signing-key.pem" just kernel
just dev
```

### Build locally with extensions

```sh
SIGNING_ARGS="--secret id=kernel_key,src=pkgs/kernel/kernel-signing-key.pem" just kernel
REGISTRY=localhost just local cloud-hypervisor
EXTENSIONS="_out/oci/cloud-hypervisor" just dev
```

#### ARM

```sh
rustup target add aarch64-unknown-linux-musl
rustup target add aarch64-unknown-uefi --toolchain nightly
ARCH=aarch64 just dev
```
