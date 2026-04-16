# syntax = docker/dockerfile-upstream:1.22.0-labs

ARG ALPINE_VERSION
ARG KERNEL_VERSION=7.0

ARG TOOLS=ghcr.io/muak-os/tools:latest

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
FROM ${TOOLS} AS tools

# ─────────────────────────────────────────────────────────────────────────────
# Assemble rootfs
# ─────────────────────────────────────────────────────────────────────────────
FROM scratch AS rootfs-base

COPY --link --from=pkg-granola    /granola     /rootfs/sbin/init
COPY --link --from=pkg-provisiond /provisiond  /rootfs/sbin/provisiond
COPY --link --from=pkg-modd       /modd        /rootfs/sbin/modd
COPY --link --from=pkg-networkd   /networkd    /rootfs/sbin/networkd
COPY --link --from=pkg-apid       /apid        /rootfs/sbin/apid
COPY --link --from=pkg-vmd        /vmd         /rootfs/sbin/vmd
COPY --link --from=pkg-timed      /timed       /rootfs/sbin/timed
COPY --link --from=pkg-consoled   /consoled    /rootfs/sbin/consoled
COPY --link --from=pkg-kernel     /lib/modules /rootfs/lib/modules

COPY --link --from=services       **/*.service /rootfs/etc/services/

# ─────────────────────────────────────────────────────────────────────────────
# Compile SELinux policy
# ─────────────────────────────────────────────────────────────────────────────
FROM docker.io/alpine:${ALPINE_VERSION} AS selinux

COPY --link --from=tools /secilc /usr/local/bin/secilc

WORKDIR /policy

COPY --link **/*.cil ./

RUN secilc -f file_contexts \
  $(find . -name '*.cil' | LC_ALL=c sort)

# ─────────────────────────────────────────────────────────────────────────────
# Create initramfs
# ─────────────────────────────────────────────────────────────────────────────
FROM ${TOOLS} AS initramfs-builder

COPY --link --from=rootfs-base /rootfs               /rootfs
COPY --link --from=selinux     /policy/policy.*      /rootfs/etc/selinux/
COPY --link --from=selinux     /policy/file_contexts /file_contexts
COPY --link --from=pkg-init    /init                 /init
COPY --link --from=pkg-kernel  /lib/modules          /lib/modules

RUN ["/ramune", "create", \
  "--init", "/init", \
  "--rootfs-dir", "/rootfs", \
  "--modules", "/lib/modules", \
  "--file-contexts", "/file_contexts", \
  "--output", "/initramfs.img"]

# ─────────────────────────────────────────────────────────────────────────────
# Final installer image
# ─────────────────────────────────────────────────────────────────────────────
FROM scratch

COPY --link --from=initramfs-builder /initramfs.img /initramfs.img
COPY --link --from=pkg-kernel        /vmlinuz       /vmlinuz
COPY --link --from=pkg-stub          /stub.efi      /stub.efi

LABEL org.opencontainers.image.title="installer"
LABEL org.opencontainers.image.description="Muak boot assets"
LABEL org.opencontainers.image.source="https://github.com/muak-os/muak"
