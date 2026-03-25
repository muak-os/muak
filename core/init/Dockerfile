# syntax = docker/dockerfile-upstream:1.22.0-labs

ARG RUST_VERSION=1.93.0
ARG ALPINE_VERSION=3.23
ARG SOURCE_DATE_EPOCH=0

# ─────────────────────────────────────────────────────────────────────────────
FROM docker.io/rust:${RUST_VERSION}-alpine${ALPINE_VERSION} AS builder

ARG SOURCE_DATE_EPOCH
ARG TARGETARCH

ARG TARGET=${TARGETARCH/amd64/x86_64}
ARG TARGET=${TARGET/arm64/aarch64}

ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}

WORKDIR /build

RUN --mount=type=cache,target=/var/cache/apk \
  --mount=type=cache,target=/etc/apk/cache \
  apk add --no-cache --no-scripts musl-dev

COPY Cargo.toml Cargo.lock ./
RUN sed -i '/members = \[/,/\]/c\members = ["core/init", "libs/kmod", "libs/kmsg", "libs/luks2"]' Cargo.toml

COPY core/init ./core/init
COPY libs/kmod ./libs/kmod
COPY libs/kmsg ./libs/kmsg
COPY libs/luks2 ./libs/luks2

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/build/target \
  <<EOF
set -euo pipefail

RUST_TARGET="${TARGET}-unknown-linux-musl"
cargo build --release --target ${RUST_TARGET} -p init
cp target/${RUST_TARGET}/release/init /init
EOF

# ─────────────────────────────────────────────────────────────────────────────
FROM scratch

COPY --link --from=builder /init /init

LABEL org.opencontainers.image.title="init"
LABEL org.opencontainers.image.description="Muak stage 1 init"
LABEL org.opencontainers.image.source="https://github.com/muak-os/muak"
