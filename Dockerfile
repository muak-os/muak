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

RUN cd granola && \
  RUSTFLAGS='-C target-feature=+crt-static -C link-arg=-fuse-ld=lld' \
  cargo build --release --target x86_64-unknown-linux-musl

RUN cd yuki && \
  RUSTFLAGS='-C target-feature=+crt-static -C link-arg=-fuse-ld=lld' \
  cargo build --release --target x86_64-unknown-linux-musl

RUN cd init && \
  RUSTFLAGS='-C target-feature=+crt-static -C link-arg=-fuse-ld=lld' \
  cargo build --release --target x86_64-unknown-linux-musl

RUN cd imager && \
  RUSTFLAGS='-C target-feature=+crt-static -C link-arg=-fuse-ld=lld' \
  cargo build --release --target x86_64-unknown-linux-musl

RUN cd stub && \
  cargo build --release --target x86_64-unknown-uefi

# ============================================================
# Download static binaries
# ============================================================
FROM alpine:latest AS downloader

ARG BTRFS_VERSION=v6.17.1

WORKDIR /download

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

COPY --from=rust-builder /build/granola/target/x86_64-unknown-linux-musl/release/granola /rootfs/sbin/init
COPY --from=rust-builder /build/yuki/target/x86_64-unknown-linux-musl/release/yuki /rootfs/sbin/yuki
COPY --from=rust-builder /build/imager/target/x86_64-unknown-linux-musl/release/muak-imager /rootfs/sbin/muak-imager

COPY --from=downloader /download/btrfs /rootfs/sbin/btrfs
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

COPY --from=rust-builder /build/init/target/x86_64-unknown-linux-musl/release/muak-init /initramfs/init
RUN chmod +x /initramfs/init

COPY --from=squashfs-builder /rootfs.sqsh /initramfs/rootfs.sqsh

RUN find . -print0 | LC_ALL=c sort -z | \
  cpio -o -H newc --null --quiet --reproducible | \
  gzip -9 > /base-initramfs.img

# ============================================================
# Prepare kernel
# ============================================================
FROM alpine:latest AS kernel-builder

# TODO: either build or download the asset
COPY build/kernel/x86_64/linux-6.17.8/arch/x86/boot/bzImage /bzImage

# ============================================================
# Final installer
# ============================================================
FROM scratch

COPY --from=initramfs-builder /base-initramfs.img /run/install/x86_64/base-initramfs.img
COPY --from=kernel-builder /bzImage /run/install/x86_64/bzImage
COPY --from=rust-builder /build/stub/target/x86_64-unknown-uefi/release/muak-stub.efi /run/install/x86_64/stub.efi

ARG VERSION=unknown
COPY --from=rust-builder <<EOF /VERSION
${VERSION}
EOF

LABEL org.opencontainers.image.title="muak-installer"
LABEL org.opencontainers.image.description="Muak Linux boot assets"
LABEL org.opencontainers.image.version="${VERSION}"
LABEL org.opencontainers.image.source="https://github.com/Sawangg/muak"
