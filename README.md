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

## Development

You need `musl` installed on your system to build binaries from source.

Add the following targets:

```sh
rustup target add x86_64-unknown-linux-musl
rustup target add aarch64-unknown-linux-musl
rustup target add x86_64-unknown-uefi --toolchain nightly # Needs nightly (only useful when building UEFI stub)
rustup target add aarch64-unknown-uefi --toolchain nightly # Needs nightly (only useful when building UEFI stub)
```

Then build from source using the cargo workspace:

```sh
cargo build --release --target <TARGET>
cargo +nightly build --release --target <UEFI-TARGET> --features=uefi -p stub
```
