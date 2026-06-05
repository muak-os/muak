# syntax = docker/dockerfile-upstream:1.22.0-labs

ARG RUST_VERSION
ARG ALPINE_VERSION
ARG SELINUX_VERSION=3.10
ARG SOURCE_DATE_EPOCH=0

# ─────────────────────────────────────────────────────────────────────────────
FROM docker.io/alpine:${ALPINE_VERSION} AS secilc

ARG SELINUX_VERSION

RUN --mount=type=cache,target=/var/cache/apk \
  --mount=type=cache,target=/etc/apk/cache \
  apk add --no-cache --no-scripts gcc musl-dev make libsepol-dev

RUN <<EOF
set -euo pipefail
wget -qO /tmp/secilc.tar.gz \
  https://github.com/SELinuxProject/selinux/releases/download/${SELINUX_VERSION}/secilc-${SELINUX_VERSION}.tar.gz
tar -xzf /tmp/secilc.tar.gz -C /tmp
make -C /tmp/secilc-${SELINUX_VERSION} LDFLAGS="-static" secilc secil2conf secil2tree
cp /tmp/secilc-${SELINUX_VERSION}/secilc /secilc
EOF

# ─────────────────────────────────────────────────────────────────────────────
FROM docker.io/rust:${RUST_VERSION}-alpine${ALPINE_VERSION} AS deps

ARG SOURCE_DATE_EPOCH
ARG TARGETARCH

ARG TARGET=${TARGETARCH/amd64/x86_64}
ARG TARGET=${TARGET/arm64/aarch64}

ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}
ENV RUST_TARGET=${TARGET}-unknown-linux-musl

WORKDIR /build

RUN --mount=type=cache,target=/var/cache/apk \
  --mount=type=cache,target=/etc/apk/cache \
  apk add --no-cache --no-scripts musl-dev

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/usr/local/cargo/git \
  cargo install --locked cargo-chef

# ─────────────────────────────────────────────────────────────────────────────
FROM deps AS planner

COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/usr/local/cargo/git \
  cargo chef prepare --recipe-path recipe.json

# ─────────────────────────────────────────────────────────────────────────────
FROM deps AS builder

COPY --from=planner /build/recipe.json recipe.json

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/usr/local/cargo/git \
  --mount=type=cache,target=/build/target \
  cargo chef cook --release --target ${RUST_TARGET} \
  --package koci --package miso --package ramune --package yuki --package imager \
  --recipe-path recipe.json

COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/usr/local/cargo/git \
  --mount=type=cache,target=/build/target \
  <<EOF
set -euo pipefail

cargo build --release --target ${RUST_TARGET} --locked -p koci -p miso -p ramune -p yuki -p imager
cp target/${RUST_TARGET}/release/koci  /koci
cp target/${RUST_TARGET}/release/miso   /miso
cp target/${RUST_TARGET}/release/ramune /ramune
cp target/${RUST_TARGET}/release/yuki   /yuki
cp target/${RUST_TARGET}/release/imager /imager
EOF

# ─────────────────────────────────────────────────────────────────────────────
FROM scratch

COPY --link --from=builder /koci   /koci
COPY --link --from=builder /miso   /miso
COPY --link --from=builder /ramune /ramune
COPY --link --from=builder /yuki   /yuki
COPY --link --from=builder /imager /imager
COPY --link --from=secilc  /secilc /secilc

LABEL org.opencontainers.image.title="tools"
LABEL org.opencontainers.image.description="Muak build tools"
LABEL org.opencontainers.image.source="https://github.com/muak-os/muak"
