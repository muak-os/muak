# syntax = docker/dockerfile-upstream:1.20.0-labs

ARG RUST_VERSION=1.91.1
ARG ALPINE_VERSION=3.23
ARG TARGET=x86_64-unknown-linux-musl
ARG SOURCE_DATE_EPOCH=0

# ============================================================
FROM rust:${RUST_VERSION}-alpine${ALPINE_VERSION} AS builder

ARG TARGET
ARG SOURCE_DATE_EPOCH

ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}

WORKDIR /build

RUN --mount=type=cache,target=/var/cache/apk \
  --mount=type=cache,target=/etc/apk/cache \
  apk add --no-cache musl-dev protoc

COPY Cargo.toml Cargo.lock ./
RUN sed -i '/members = \[/,/\]/c\members = ["internal/modd", "internal/shared/kmod", "internal/shared/kmsg", "internal/shared/notify"]' Cargo.toml

COPY api ./api
COPY internal/modd ./internal/modd
COPY internal/shared/kmod ./internal/shared/kmod
COPY internal/shared/kmsg ./internal/shared/kmsg
COPY internal/shared/notify ./internal/shared/notify

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/build/target \
  <<EOF
set -euo pipefail
cargo build --release --target ${TARGET} -p modd
cp target/${TARGET}/release/modd /modd
EOF

# ============================================================
FROM scratch AS modd-package

COPY --link --from=builder /modd /modd

LABEL org.opencontainers.image.title="modd"
LABEL org.opencontainers.image.description="Muak module daemon for dynamic kernel module loading"
LABEL org.opencontainers.image.source="https://github.com/Sawangg/muak"
