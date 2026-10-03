# Contributing

## Prerequisites

You need the following tools installed on your host system: `git`, `musl`, `rustup`, `just`, `cargo-nextest` and `docker` or `podman`

## Quick Start

Local QEMU development uses two addresses for the same registry:

- `localhost:5000` from the host, for `just dev` and other pushes
- `10.0.2.2:5000` from inside the QEMU guest, for installs and `just e2e`

`REGISTRY` controls where Muak images are pushed. The toolchain image is pulled by default from ghcr unless you explicitly set `TOOLCHAIN`.

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
REGISTRY="localhost:5000" just mirror linux latest
REGISTRY="localhost:5000" just mirror stub latest
TAG="v0.0.1" REGISTRY="localhost:5000" just build --release
TAG="v0.0.1" REGISTRY="localhost:5000" just installer
TAG="v0.0.1" REGISTRY="localhost:5000" just catalog \
  --set kernels/muak-os/linux=linux@latest \
  --set stub=muak-os/stub@stub@latest
REGISTRY="localhost:5000" just dev # Uses the default toolchain
just start

REGISTRY="10.0.2.2:5000" just e2e
```

Every run after that is plain `REGISTRY="10.0.2.2:5000" just dev` to iterate.

### Local Toolchain Image

```sh
REGISTRY="localhost:5000" just oci toolchain
TOOLCHAIN="localhost:5000/toolchain:latest" REGISTRY="localhost:5000" just dev
```

### ARM

```sh
ARCH=aarch64 REGISTRY="localhost:5000" just dev
```
