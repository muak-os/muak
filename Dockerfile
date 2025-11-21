# ============================================================
# Build Rust binaries
# ============================================================
FROM rust:1.91.1-alpine3.22 AS rust-builder

WORKDIR /build

RUN apk add --no-cache \
  clang \
  lld \
  protoc

COPY api ./api
COPY internal/granola ./granola
COPY internal/yuki ./yuki
COPY internal/init ./init
COPY internal/imager ./imager
COPY internal/stub ./stub

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/build/granola/target \
  cd granola && \
  RUSTFLAGS='-C target-feature=+crt-static -C link-arg=-fuse-ld=lld' \
  cargo build --release --target x86_64-unknown-linux-musl && \
  cp target/x86_64-unknown-linux-musl/release/granola /granola

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/build/yuki/target \
  cd yuki && \
  RUSTFLAGS='-C target-feature=+crt-static -C link-arg=-fuse-ld=lld' \
  cargo build --release --target x86_64-unknown-linux-musl && \
  cp target/x86_64-unknown-linux-musl/release/yuki /yuki

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/build/init/target \
  cd init && \
  RUSTFLAGS='-C target-feature=+crt-static -C link-arg=-fuse-ld=lld' \
  cargo build --release --target x86_64-unknown-linux-musl && \
  cp target/x86_64-unknown-linux-musl/release/muak-init /init

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/build/imager/target \
  cd imager && \
  RUSTFLAGS='-C target-feature=+crt-static -C link-arg=-fuse-ld=lld' \
  cargo build --release --target x86_64-unknown-linux-musl && \
  cp target/x86_64-unknown-linux-musl/release/imager /imager

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/build/stub/target \
  cd stub && \
  cargo build --release --target x86_64-unknown-uefi && \
  cp target/x86_64-unknown-uefi/release/muak-stub.efi /stub-bin.efi

# ============================================================
# Download static binaries
# ============================================================
FROM alpine:latest AS tools

ARG BTRFS_VERSION=v6.17.1

WORKDIR /tools

RUN apk add --no-cache curl && \
  curl -fsSL "https://github.com/kdave/btrfs-progs/releases/download/${BTRFS_VERSION}/btrfs.box.static" \
  -o btrfs && \
  chmod +x btrfs

# ============================================================
# Create base rootfs
# ============================================================
FROM alpine:latest AS rootfs-base

WORKDIR /rootfs

RUN mkdir -p \
  sbin \
  dev \
  proc \
  sys \
  run \
  etc \
  tmp \
  mnt

COPY --from=rust-builder /granola /rootfs/sbin/init
COPY --from=rust-builder /yuki /rootfs/sbin/yuki
COPY --from=rust-builder /imager /rootfs/sbin/imager

COPY --from=tools /tools/btrfs /rootfs/sbin/btrfs
RUN ln -s btrfs /rootfs/sbin/mkfs.btrfs

RUN echo "nameserver 9.9.9.9" > /rootfs/run/resolv.conf && \
  ln -sf /run/resolv.conf /rootfs/etc/resolv.conf

# ============================================================
# Create squashfs
# ============================================================
FROM alpine:latest AS squashfs-builder

RUN apk add --no-cache squashfs-tools

COPY --from=rootfs-base /rootfs /rootfs

RUN SOURCE_DATE_EPOCH=0 mksquashfs /rootfs /rootfs.sqsh \
  -comp gzip \
  -b 1M \
  -noappend \
  -no-progress

# ============================================================
# Create base initramfs
# ============================================================
FROM alpine:latest AS initramfs-builder

RUN apk add --no-cache cpio gzip

WORKDIR /initramfs

COPY --from=rust-builder /init /initramfs/init
RUN chmod +x /initramfs/init

COPY --from=squashfs-builder /rootfs.sqsh /initramfs/rootfs.sqsh

RUN find . -print0 | LC_ALL=c sort -z | \
  cpio -o -H newc --null --quiet --reproducible | \
  gzip -9n > /base-initramfs.img

# ============================================================
# Use pre-built kernel package
# ============================================================
ARG PKG_KERNEL=ghcr.io/sawangg/muak/kernel:6.17.8
FROM ${PKG_KERNEL} AS kernel-package

# ============================================================
# Final installer
# ============================================================
FROM scratch

COPY --from=initramfs-builder /base-initramfs.img /run/install/x86_64/base-initramfs.img
COPY --from=kernel-package /bzImage /run/install/x86_64/bzImage
COPY --from=rust-builder /stub-bin.efi /run/install/x86_64/stub.efi

ARG VERSION=unknown
COPY --from=rust-builder <<EOF /VERSION
${VERSION}
EOF

LABEL org.opencontainers.image.title="muak-installer"
LABEL org.opencontainers.image.description="Muak Linux boot assets"
LABEL org.opencontainers.image.version="${VERSION}"
LABEL org.opencontainers.image.source="https://github.com/Sawangg/muak"
