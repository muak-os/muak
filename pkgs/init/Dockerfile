# syntax = docker/dockerfile-upstream:1.20.0-labs

ARG RUST_VERSION=1.91.1
ARG ALPINE_VERSION=3.22
ARG TARGET=x86_64-unknown-linux-musl
ARG SOURCE_DATE_EPOCH=0

# ============================================================
FROM rust:${RUST_VERSION}-alpine${ALPINE_VERSION} AS builder

ARG TARGET
ARG SOURCE_DATE_EPOCH

ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}

WORKDIR /build

RUN apk add --no-cache clang lld

COPY Cargo.toml Cargo.lock ./
COPY internal/init ./internal/init

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/build/target \
  <<EOF
set -euo pipefail
RUSTFLAGS='-C target-feature=+crt-static -C link-arg=-fuse-ld=lld' \
  cargo build --release --target ${TARGET} -p muak-init
cp target/${TARGET}/release/muak-init /init
EOF

# ============================================================
FROM scratch AS init-package

COPY --from=builder /init /init

LABEL org.opencontainers.image.title="init"
LABEL org.opencontainers.image.description="Muak stage 1 init"
LABEL org.opencontainers.image.source="https://github.com/Sawangg/muak"
