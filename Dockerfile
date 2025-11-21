# syntax = docker/dockerfile-upstream:1.20.0-labs

ARG RUST_VERSION=1.91.1
ARG ALPINE_VERSION=3.22
ARG BTRFS_VERSION=v6.17.1
ARG KERNEL_VERSION=6.17.8
ARG COMPRESSION_LEVEL=9
ARG TARGET_MUSL=x86_64-unknown-linux-musl
ARG TARGET_UEFI=x86_64-unknown-uefi
ARG SOURCE_DATE_EPOCH=0

ARG PKG_KERNEL=ghcr.io/sawangg/muak/kernel:${KERNEL_VERSION}

# ============================================================
# Rust base image with dependencies
# ============================================================
FROM rust:${RUST_VERSION}-alpine${ALPINE_VERSION} AS rust-base

WORKDIR /build

RUN apk add --no-cache \
  clang \
  lld \
  protoc

# ============================================================
# Pre-fetch Rust dependencies
# ============================================================
FROM rust-base AS rust-deps

COPY Cargo.toml Cargo.lock ./

COPY cli/Cargo.toml ./cli/
COPY internal/granola/Cargo.toml ./internal/granola/
COPY internal/yuki/Cargo.toml ./internal/yuki/
COPY internal/init/Cargo.toml ./internal/init/
COPY internal/imager/Cargo.toml ./internal/imager/

COPY cli/build.rs ./cli/
COPY internal/granola/build.rs ./internal/granola/
COPY api ./api

RUN <<EOF
set -euo pipefail
mkdir -p cli/src internal/granola/src internal/yuki/src internal/init/src internal/imager/src
echo "fn main() {}" > cli/src/main.rs
echo "fn main() {}" > internal/granola/src/main.rs
echo "fn main() {}" > internal/yuki/src/main.rs
echo "fn main() {}" > internal/init/src/main.rs
echo "fn main() {}" > internal/imager/src/main.rs
EOF

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/build/target \
  cargo fetch --locked

COPY internal/stub/Cargo.toml internal/stub/Cargo.lock ./stub/
COPY internal/stub/rust-toolchain.toml ./stub/

RUN <<EOF
set -euo pipefail
mkdir -p stub/src
echo "fn main() {}" > stub/src/main.rs
EOF

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/build/stub/target \
  cd stub && cargo fetch --locked

# ============================================================
# Build granola binary
# ============================================================
FROM rust-deps AS granola-build

ARG TARGET_MUSL
ARG SOURCE_DATE_EPOCH

ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}

COPY api ./api
COPY internal/granola ./internal/granola

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/build/target \
  <<EOF
set -euo pipefail
RUSTFLAGS='-C target-feature=+crt-static -C link-arg=-fuse-ld=lld' \
  cargo build --release --target ${TARGET_MUSL} -p granola
cp target/${TARGET_MUSL}/release/granola /granola
EOF

# ============================================================
# Build yuki binary
# ============================================================
FROM rust-deps AS yuki-build

ARG TARGET_MUSL
ARG SOURCE_DATE_EPOCH

ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}

COPY internal/yuki ./internal/yuki

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/build/target \
  <<EOF
set -euo pipefail
RUSTFLAGS='-C target-feature=+crt-static -C link-arg=-fuse-ld=lld' \
  cargo build --release --target ${TARGET_MUSL} -p yuki
cp target/${TARGET_MUSL}/release/yuki /yuki
EOF

# ============================================================
# Build init binary
# ============================================================
FROM rust-deps AS init-build

ARG TARGET_MUSL
ARG SOURCE_DATE_EPOCH

ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}

COPY internal/init ./internal/init

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/build/target \
  <<EOF
set -euo pipefail
RUSTFLAGS='-C target-feature=+crt-static -C link-arg=-fuse-ld=lld' \
  cargo build --release --target ${TARGET_MUSL} -p muak-init
cp target/${TARGET_MUSL}/release/muak-init /init
EOF

# ============================================================
# Build imager binary
# ============================================================
FROM rust-deps AS imager-build

ARG TARGET_MUSL
ARG SOURCE_DATE_EPOCH

ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}

COPY internal/imager ./internal/imager

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/build/target \
  <<EOF
set -euo pipefail
RUSTFLAGS='-C target-feature=+crt-static -C link-arg=-fuse-ld=lld' \
  cargo build --release --target ${TARGET_MUSL} -p imager
cp target/${TARGET_MUSL}/release/imager /imager
EOF

# ============================================================
# Build stub binary
# ============================================================
FROM rust-deps AS stub-build

ARG TARGET_UEFI
ARG SOURCE_DATE_EPOCH

ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}

COPY internal/stub ./stub

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/build/stub/target \
  <<EOF
set -euo pipefail
cd stub
cargo build --release --target ${TARGET_UEFI}
cp target/${TARGET_UEFI}/release/muak-stub.efi /stub.efi
EOF

# ============================================================
# Collect all binaries
# ============================================================
FROM scratch AS rust-binaries

COPY --link --from=granola-build /granola /granola
COPY --link --from=yuki-build /yuki /yuki
COPY --link --from=init-build /init /init
COPY --link --from=imager-build /imager /imager
COPY --link --from=stub-build /stub.efi /stub.efi

# ============================================================
# Download static binaries
# ============================================================
FROM alpine:${ALPINE_VERSION} AS tools

ARG BTRFS_VERSION

WORKDIR /tools

RUN <<EOF
set -euo pipefail
apk add --no-cache curl
curl -fsSL "https://github.com/kdave/btrfs-progs/releases/download/${BTRFS_VERSION}/btrfs.box.static" \
  -o btrfs
chmod +x btrfs
EOF

# ============================================================
# Create base rootfs structure
# ============================================================
FROM alpine:${ALPINE_VERSION} AS rootfs-structure

ARG SOURCE_DATE_EPOCH

ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}

WORKDIR /rootfs

RUN <<EOF
set -euo pipefail
mkdir -p sbin dev proc sys run etc tmp mnt
echo "nameserver 9.9.9.9" > run/resolv.conf
ln -sf /run/resolv.conf etc/resolv.conf
EOF

# ============================================================
# Assemble complete rootfs
# ============================================================
FROM rootfs-structure AS rootfs-base

ARG SOURCE_DATE_EPOCH

ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}

COPY --link --from=rust-binaries /granola /rootfs/sbin/init
COPY --link --from=rust-binaries /yuki /rootfs/sbin/yuki
COPY --link --from=rust-binaries /imager /rootfs/sbin/imager

COPY --link --from=tools /tools/btrfs /rootfs/sbin/btrfs

RUN ln -s btrfs /rootfs/sbin/mkfs.btrfs

RUN find /rootfs -print0 | xargs -0r touch --no-dereference --date="@${SOURCE_DATE_EPOCH}"

# ============================================================
# Create squashfs
# ============================================================
FROM alpine:${ALPINE_VERSION} AS squashfs-builder

ARG SOURCE_DATE_EPOCH
ARG COMPRESSION_LEVEL

ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}

RUN apk add --no-cache squashfs-tools

COPY --link --from=rootfs-base /rootfs /rootfs

RUN <<EOF
set -euo pipefail
mksquashfs /rootfs /rootfs.sqsh \
  -comp gzip \
  -Xcompression-level ${COMPRESSION_LEVEL} \
  -b 1M \
  -noappend \
  -no-progress
EOF

# ============================================================
# Create base initramfs
# ============================================================
FROM alpine:${ALPINE_VERSION} AS initramfs-builder

ARG SOURCE_DATE_EPOCH
ARG COMPRESSION_LEVEL

ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}

RUN apk add --no-cache cpio gzip

WORKDIR /initramfs

COPY --link --from=rust-binaries /init /initramfs/init
RUN chmod +x /initramfs/init

COPY --link --from=squashfs-builder /rootfs.sqsh /initramfs/rootfs.sqsh

RUN <<EOF
set -euo pipefail
find . -print0 | xargs -0r touch --no-dereference --date="@${SOURCE_DATE_EPOCH}"
find . -print0 | LC_ALL=c sort -z | \
  cpio -o -H newc --null --quiet --reproducible | \
  gzip -${COMPRESSION_LEVEL}n > /base-initramfs.img
EOF

# ============================================================
# Use pre-built kernel package
# ============================================================
FROM ${PKG_KERNEL} AS kernel-package

# ============================================================
# Final installer image
# ============================================================
FROM scratch

COPY --link --from=initramfs-builder /base-initramfs.img /run/install/x86_64/base-initramfs.img
COPY --link --from=kernel-package /bzImage /run/install/x86_64/bzImage
COPY --link --from=rust-binaries /stub.efi /run/install/x86_64/stub.efi

ARG VERSION=unknown
COPY --from=rust-binaries <<EOF /VERSION
${VERSION}
EOF

LABEL org.opencontainers.image.title="installer"
LABEL org.opencontainers.image.description="Muak Linux boot assets"
LABEL org.opencontainers.image.version="${VERSION}"
LABEL org.opencontainers.image.source="https://github.com/Sawangg/muak"
