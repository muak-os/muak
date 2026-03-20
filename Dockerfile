# syntax = docker/dockerfile-upstream:1.20.0-labs

ARG ALPINE_VERSION=3.23
ARG KERNEL_VERSION=6.19.8
ARG COMPRESSION_LEVEL=19
ARG SOURCE_DATE_EPOCH=0

ARG PKG_KERNEL=ghcr.io/muak-os/pkgs/kernel:${KERNEL_VERSION}
ARG PKG_GRANOLA=ghcr.io/muak-os/pkgs/granola:latest
ARG PKG_PROVISIOND=ghcr.io/muak-os/pkgs/provisiond:latest
ARG PKG_MODD=ghcr.io/muak-os/pkgs/modd:latest
ARG PKG_NETWORKD=ghcr.io/muak-os/pkgs/networkd:latest
ARG PKG_APID=ghcr.io/muak-os/pkgs/apid:latest
ARG PKG_VMD=ghcr.io/muak-os/pkgs/vmd:latest
ARG PKG_TIMED=ghcr.io/muak-os/pkgs/timed:latest
ARG PKG_CONSOLED=ghcr.io/muak-os/pkgs/consoled:latest
ARG PKG_INIT=ghcr.io/muak-os/pkgs/init:latest
ARG PKG_STUB=ghcr.io/muak-os/pkgs/stub:latest

# ─────────────────────────────────────────────────────────────────────────────
# Import packages
# ─────────────────────────────────────────────────────────────────────────────
FROM ${PKG_GRANOLA} AS pkg-granola
FROM ${PKG_PROVISIOND} AS pkg-provisiond
FROM ${PKG_MODD} AS pkg-modd
FROM ${PKG_NETWORKD} AS pkg-networkd
FROM ${PKG_APID} AS pkg-apid
FROM ${PKG_VMD} AS pkg-vmd
FROM ${PKG_TIMED} AS pkg-timed
FROM ${PKG_CONSOLED} AS pkg-consoled
FROM ${PKG_INIT} AS pkg-init
FROM ${PKG_STUB} AS pkg-stub
FROM ${PKG_KERNEL} AS pkg-kernel

# ─────────────────────────────────────────────────────────────────────────────
# Create SELinux policy
# ─────────────────────────────────────────────────────────────────────────────
FROM docker.io/debian:trixie-slim AS selinux-policy-builder

RUN apt-get update && apt-get install -y --no-install-recommends \
  secilc \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /policy

COPY --link **/*.cil ./

RUN secilc -c 34 -o policy.34 -f /dev/null \
  $(find . -name '*.cil' | LC_ALL=c sort)

# ─────────────────────────────────────────────────────────────────────────────
# Create base rootfs structure
# ─────────────────────────────────────────────────────────────────────────────
FROM docker.io/alpine:${ALPINE_VERSION} AS rootfs-structure

ARG SOURCE_DATE_EPOCH

ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}

WORKDIR /rootfs

RUN <<EOF
set -euo pipefail
mkdir -p sbin dev proc sys run etc/services etc/selinux lib/modules
ln -sf /run/resolv.conf etc/resolv.conf
EOF

# ─────────────────────────────────────────────────────────────────────────────
# Assemble complete rootfs
# ─────────────────────────────────────────────────────────────────────────────
FROM rootfs-structure AS rootfs-base

ARG SOURCE_DATE_EPOCH

ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}

COPY --link --from=pkg-granola /granola /rootfs/sbin/init
COPY --link --from=pkg-provisiond /provisiond /rootfs/sbin/provisiond
COPY --link --from=pkg-modd /modd /rootfs/sbin/modd
COPY --link --from=pkg-networkd /networkd /rootfs/sbin/networkd
COPY --link --from=pkg-apid /apid /rootfs/sbin/apid
COPY --link --from=pkg-vmd /vmd /rootfs/sbin/vmd
COPY --link --from=pkg-timed /timed /rootfs/sbin/timed
COPY --link --from=pkg-consoled /consoled /rootfs/sbin/consoled

COPY --link --from=pkg-kernel /lib/modules /rootfs/lib/modules

COPY --link --from=services services/*/*.service /rootfs/etc/services/

RUN find /rootfs -print0 | xargs -0r touch --no-dereference --date="@${SOURCE_DATE_EPOCH}"

# ─────────────────────────────────────────────────────────────────────────────
# Create squashfs
# ─────────────────────────────────────────────────────────────────────────────
FROM docker.io/alpine:${ALPINE_VERSION} AS squashfs-builder

ARG SOURCE_DATE_EPOCH
ARG COMPRESSION_LEVEL

ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}

RUN apk add --no-cache squashfs-tools

COPY --link --from=rootfs-base /rootfs /rootfs
COPY --link --from=selinux-policy-builder /policy/policy.34 /rootfs/etc/selinux/policy.34

COPY --link core/init/core.fc /fc/core.fc
COPY --link services/*/*.fc /fc/

RUN <<EOF
set -euo pipefail

cat /fc/*.fc > /tmp/file-contexts.txt

default="system_u:object_r:file_t:s0"
: > /tmp/selinux-labels.pf

cd /rootfs
find . -mindepth 1 | sed 's|^\./||' | LC_ALL=C sort | while read -r path; do
    ctx=$(awk -v p="$path" '
        $1 == p { print $2; exit }
        index($1, "*") && p ~ "^" substr($1, 1, length($1)-1) { print $2; exit }
    ' /tmp/file-contexts.txt)
    printf '%s x security.selinux=%s\n' "$path" "${ctx:-$default}" >> /tmp/selinux-labels.pf
done || true

mksquashfs /rootfs /rootfs.sqsh \
  -comp zstd \
  -Xcompression-level ${COMPRESSION_LEVEL} \
  -b 1M \
  -noappend \
  -no-progress \
  -pf /tmp/selinux-labels.pf
EOF

# ─────────────────────────────────────────────────────────────────────────────
# Create base initramfs
# ─────────────────────────────────────────────────────────────────────────────
FROM docker.io/alpine:${ALPINE_VERSION} AS initramfs-builder

ARG SOURCE_DATE_EPOCH
ARG COMPRESSION_LEVEL

ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}

RUN apk add --no-cache cpio zstd

WORKDIR /initramfs

COPY --link --chmod=755 --from=pkg-init /init /initramfs/init
COPY --link --from=squashfs-builder /rootfs.sqsh /initramfs/rootfs.sqsh
COPY --link --from=pkg-kernel /lib/modules /initramfs/lib/modules

RUN <<EOF
set -euo pipefail
find . -print0 | xargs -0r touch --no-dereference --date="@${SOURCE_DATE_EPOCH}"
find . -print0 | LC_ALL=c sort -z | \
  cpio -o -H newc --null --quiet --reproducible | \
  zstd -${COMPRESSION_LEVEL} -T0 > /base-initramfs.img
EOF

# ─────────────────────────────────────────────────────────────────────────────
# Final installer image
# ─────────────────────────────────────────────────────────────────────────────
FROM scratch

COPY --link --from=initramfs-builder /base-initramfs.img /base-initramfs.img
COPY --link --from=pkg-kernel /vmlinuz /vmlinuz
COPY --link --from=pkg-stub /stub.efi /stub.efi

LABEL org.opencontainers.image.title="installer"
LABEL org.opencontainers.image.description="Muak Linux boot assets"
LABEL org.opencontainers.image.source="https://github.com/muak-os/muak"
