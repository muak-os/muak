# syntax = docker/dockerfile-upstream:1.20.0-labs

ARG RUST_VERSION=1.91.1
ARG ALPINE_VERSION=3.23
ARG SOURCE_DATE_EPOCH=0

# ============================================================
FROM rust:${RUST_VERSION}-alpine${ALPINE_VERSION} AS builder

ARG SOURCE_DATE_EPOCH
ARG TARGETARCH

ARG TARGET=${TARGETARCH/amd64/x86_64}
ARG TARGET=${TARGET/arm64/aarch64}

ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}

WORKDIR /build

RUN --mount=type=cache,target=/var/cache/apk \
  --mount=type=cache,target=/etc/apk/cache \
  apk add --no-cache musl-dev protoc

COPY Cargo.toml Cargo.lock ./
RUN sed -i '/members = \[/,/\]/c\members = ["internal/vmd", "internal/shared/kmsg", "internal/shared/notify", "internal/shared/sysconfig"]' Cargo.toml

COPY api ./api
COPY internal/default.toml ./internal/default.toml
COPY internal/vmd ./internal/vmd
COPY internal/shared/kmsg ./internal/shared/kmsg
COPY internal/shared/notify ./internal/shared/notify
COPY internal/shared/sysconfig ./internal/shared/sysconfig

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/build/target \
  <<EOF
set -euo pipefail

RUST_TARGET="${TARGET}-unknown-linux-musl"
cargo build --release --target ${RUST_TARGET} -p vmd
cp target/${RUST_TARGET}/release/vmd /vmd
EOF

# ============================================================
FROM scratch AS vmd-package

COPY --link --from=builder /vmd /vmd

LABEL org.opencontainers.image.title="vmd"
LABEL org.opencontainers.image.description="Muak VM management daemon"
LABEL org.opencontainers.image.source="https://github.com/Sawangg/muak"
