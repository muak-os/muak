# Contributing

## Prerequisites

For local Rust development, install `git`, a C compiler, `rustup`, `just`,
`cargo-nextest` and `jq`. The repository pins its Rust toolchain, components and
targets in `rust-toolchain.toml`; use that toolchain for builds and checks.

Building and booting Muak also requires `musl`, `podman` (the default container
runtime) or Docker with Buildx, `qemu-system-x86_64`, `qemu-img` and access to
`/dev/kvm`. Set `CONTAINER_RUNTIME=docker` when using Docker.

## Working on Wizard

Wizard can be built and tested without a container runtime or a running Muak VM.
Its integration tests start their own local mock registry.

```sh
cargo run --locked -p wizard -- --help
just test wizard
just lint wizard
```

To limit host compilation and test memory use, set the parallelism before running
these commands:

```sh
export CARGO_BUILD_JOBS=2
export CARGO_INCREMENTAL=0
export NEXTEST_TEST_THREADS=2
```

The `just test wizard` recipe runs both tests and doctests. `just lint wizard`
checks workspace formatting and runs Clippy for wizard and its test targets.
With `cargo-hack` installed, `just features wizard` also checks its feature
combinations. These checks do not boot or validate the full OS.

## Quick Start

Local QEMU development uses two addresses for the same registry:

- `localhost:5000` from the host, for `just dev` and other pushes
- `10.0.2.2:5000` from inside the QEMU guest, for installs and `just e2e`

`REGISTRY` controls where Muak images are pushed. The toolchain image is pulled by default from ghcr unless you explicitly set `TOOLCHAIN`.

Start a local registry once (use `podman start registry` if it already exists):

```sh
podman run -d -p 5000:5000 --name registry docker.io/library/registry:3
```

The registry serves plain HTTP. For Podman, add the following entries to
`~/.config/containers/registries.conf`, preserving any existing settings, so the
client accepts pushes and pulls:

```toml
[[registry]]
prefix = "localhost:5000"
insecure = true
location = "localhost:5000"

[[registry]]
prefix = "10.0.2.2:5000"
insecure = true
location = "10.0.2.2:5000"
```

`just sign` (also called by `just dev`) needs an image signing key. By default it
reads `signature.key` from the repository root; set `SIGNATURE` to use another
path. Private keys are not committed to the repository.

Mirror the kernel and stub, then build and publish the initial local catalog:

```sh
REGISTRY="localhost:5000" just mirror linux v7.2.11
REGISTRY="localhost:5000" just mirror stub v1.0.1-beta
REGISTRY="localhost:5000" just build --release
REGISTRY="localhost:5000" just installer
REGISTRY="localhost:5000" just sign
REGISTRY="localhost:5000" just catalog \
  --set kernels/muak-os/linux=linux@v7.2.11 \
  --set stub=muak-os/stub@stub@v1.0.1-beta
REGISTRY="localhost:5000" just artifacts iso
just start

REGISTRY="10.0.2.2:5000" just e2e
```

For subsequent builds, `just dev` chains the release build, installer, signing,
catalog publication and ISO generation. Keep using the host registry address:

```sh
REGISTRY="localhost:5000" just dev
just start
```

`just start` boots `_out/muak.iso`, fetches missing OVMF firmware and forwards the
guest API to `127.0.0.1:50051`. The QEMU helpers currently support x86_64 only.

### Local Toolchain Image

By default, artifact generation runs the wizard from the published toolchain
image. To include local changes to wizard or the other toolchain programs, build
and select a local image. Rebuild it after changing those programs:

```sh
REGISTRY="localhost:5000" just oci toolchain
TOOLCHAIN="localhost:5000/toolchain:0.0.0-dev" REGISTRY="localhost:5000" just dev
```

### ARM

```sh
ARCH=aarch64 REGISTRY="localhost:5000" just dev
```

## Pull Requests

Create a focused branch from the current `origin/master` and open a pull request
targeting `master`. Keep unrelated changes in separate pull requests. Describe the
resulting behavior, include a usage example when relevant, and list the checks
you ran along with any validation that remains pending.

For wizard changes, start with the package checks above. CI additionally checks
workspace lint, tests, feature combinations, documentation, spelling and
dependency policies. Changes affecting boot or provisioning also need full VM
validation.
