# Contributing

## Prerequisites

You need the following tools installed on your host system: `git`, `musl`, `rustup`, `just`, `cargo-nextest` and `docker` or `podman`

Add the following targets:

```sh
rustup target add x86_64-unknown-linux-musl
rustup component add rust-analyzer
rustup target add x86_64-unknown-uefi --toolchain nightly
rustup component add rust-analyzer --toolchain nightly
```

## Quick Start

```sh
podman run -d -p 192.168.100.1:5000:5000 --name registry docker.io/library/registry:3
# Create a signing key for the kernel and place it in core/kernel/
KERNEL_SIGNING="--secret id=kernel_key,src=core/kernel/kernel-signing-key.pem" REGISTRY="localhost" just kernel
just extract --image localhost/kernel:latest
REGISTRY="192.168.100.1:5000" PUSH="true" just dev
REGISTRY="192.168.100.1:5000" just e2e
```

## Build locally with extensions

```sh
SIGNING_ARGS="--secret id=kernel_key,src=core/kernel/kernel-signing-key.pem" just kernel
REGISTRY=localhost just local cloud-hypervisor
EXTENSIONS="_out/oci/cloud-hypervisor" just dev
```

### ARM

```sh
rustup target add aarch64-unknown-linux-musl
rustup target add aarch64-unknown-uefi --toolchain nightly
ARCH=aarch64 REGISTRY="192.168.100.1:5000" PUSH="true" just dev
```
