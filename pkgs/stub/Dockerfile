# syntax = docker/dockerfile-upstream:1.20.0-labs

ARG RUST_VERSION=1.91.1
ARG ALPINE_VERSION=3.22
ARG TARGET_UEFI=x86_64-unknown-uefi
ARG SOURCE_DATE_EPOCH=0

# ============================================================
FROM rust:${RUST_VERSION}-alpine${ALPINE_VERSION} AS builder

ARG TARGET_UEFI
ARG SOURCE_DATE_EPOCH

ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}

WORKDIR /build

RUN --mount=type=cache,target=/var/cache/apk \
  --mount=type=cache,target=/etc/apk/cache \
  apk add --no-cache clang lld

COPY internal/stub/Cargo.toml internal/stub/Cargo.lock ./
COPY internal/stub/rust-toolchain.toml ./
COPY internal/stub/src ./src

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/build/target \
  <<EOF
set -euo pipefail
cargo build --release --target ${TARGET_UEFI}
cp target/${TARGET_UEFI}/release/muak-stub.efi /stub.efi
EOF

# ============================================================
FROM scratch AS stub-package

COPY --link --from=builder /stub.efi /stub.efi

LABEL org.opencontainers.image.title="muak-stub"
LABEL org.opencontainers.image.description="Muak UEFI stub"
LABEL org.opencontainers.image.source="https://github.com/Sawangg/muak"
