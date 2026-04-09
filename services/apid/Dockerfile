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
RUN sed -i '/members = \[/,/\]/c\members = ["services/apid", "core/granola", "libs/kmsg", "libs/config", "libs/pki"]' Cargo.toml

COPY api ./api
COPY libs/config/default.toml ./libs/config/default.toml
COPY services/apid ./services/apid
COPY core/granola ./core/granola
COPY libs/kmsg ./libs/kmsg
COPY libs/config ./libs/config
COPY libs/pki ./libs/pki

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/build/target \
  <<EOF
set -euo pipefail

RUST_TARGET="${TARGET}-unknown-linux-musl"
cargo build --release --target ${RUST_TARGET} -p apid
cp target/${RUST_TARGET}/release/apid /apid
EOF

# ─────────────────────────────────────────────────────────────────────────────
FROM scratch

COPY --link --from=builder /apid                           /apid
COPY --link                services/apid/apid.service      /etc/services/apid.service

LABEL org.opencontainers.image.title="apid"
LABEL org.opencontainers.image.description="Muak external gRPC API daemon"
LABEL org.opencontainers.image.source="https://github.com/muak-os/muak"
