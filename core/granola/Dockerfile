# syntax = docker/dockerfile-upstream:1.22.0-labs

ARG RUST_VERSION
ARG ALPINE_VERSION
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
RUN sed -i '/members = \[/,/\]/c\members = ["core/granola", "libs/kmsg", "libs/granola-macros", "libs/config"]' Cargo.toml

COPY api ./api
COPY libs/config/default.toml ./libs/config/default.toml
COPY core/granola ./core/granola
COPY libs/kmsg ./libs/kmsg
COPY libs/granola-macros ./libs/granola-macros
COPY libs/config ./libs/config

COPY core/kernel/cmdline-amd64.txt ./core/kernel/cmdline-amd64.txt
COPY core/kernel/cmdline-arm64.txt ./core/kernel/cmdline-arm64.txt

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/usr/local/cargo/git \
  --mount=type=cache,target=/build/target \
  <<EOF
set -euo pipefail

RUST_TARGET="${TARGET}-unknown-linux-musl"
cargo build --release --target ${RUST_TARGET} -p granola
cp target/${RUST_TARGET}/release/granola /granola
EOF

# ─────────────────────────────────────────────────────────────────────────────
FROM scratch

COPY --link --from=builder /granola /granola

LABEL org.opencontainers.image.title="granola"
LABEL org.opencontainers.image.description="Muak stage 2 init system"
LABEL org.opencontainers.image.source="https://github.com/muak-os/muak"
