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

Local QEMU development uses two addresses for the same registry:

- `localhost:5000` from the host, for `just dev` and other pushes
- `10.0.2.2:5000` from inside the QEMU guest, for installs and `just e2e`

`REGISTRY` controls where Muak images are pushed. The tools image still defaults to `ghcr.io/muak-os/tools:<tag>` unless you explicitly set `TOOLS`.

Start a local registry:

```sh
podman run -d -p 5000:5000 --name registry docker.io/library/registry:3
```

The registry serves plain HTTP, so it must be told it is insecure or pushes
and pulls fail. Mark it as insecure in your user config so the client accepts it:

```sh
mkdir -p ~/.config/containers
cat > ~/.config/containers/registries.conf <<'EOF'
[[registry]]
prefix = "localhost:5000"
insecure = true
location = "localhost:5000"

[[registry]]
prefix = "10.0.2.2:5000"
insecure = true
location = "10.0.2.2:5000"
EOF
```

```sh
REGISTRY="localhost:5000" PUSH="true" just dev
just start

REGISTRY="10.0.2.2:5000" just e2e
```

### Local tool image

```sh
REGISTRY="localhost:5000" PUSH="true" just oci tools
TOOLS="localhost:5000/tools:latest" REGISTRY="localhost:5000" PUSH="true" just dev
```

### ARM

```sh
rustup target add aarch64-unknown-linux-musl
rustup target add aarch64-unknown-uefi --toolchain nightly
ARCH=aarch64 REGISTRY="localhost:5000" PUSH="true" just dev
```
