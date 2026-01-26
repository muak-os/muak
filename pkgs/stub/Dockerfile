# syntax = docker/dockerfile-upstream:1.20.0-labs

ARG RUST_VERSION=1.91.1
ARG ALPINE_VERSION=3.23
ARG SOURCE_DATE_EPOCH=0

# ============================================================
FROM rust:${RUST_VERSION}-alpine${ALPINE_VERSION} AS builder

ARG SOURCE_DATE_EPOCH
ARG TARGETARCH

# Map Docker TARGETARCH to UEFI target
ARG TARGET=${TARGETARCH/amd64/x86_64}
ARG TARGET=${TARGET/arm64/aarch64}

ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}

WORKDIR /build

RUN --mount=type=cache,target=/var/cache/apk \
  --mount=type=cache,target=/etc/apk/cache \
  apk add --no-cache musl-dev

COPY Cargo.toml Cargo.lock ./
RUN sed -i '/members = \[/,/\]/c\members = ["internal/stub"]' Cargo.toml

COPY internal/stub ./internal/stub

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/build/target \
  <<EOF
set -euo pipefail

UEFI_TARGET="${TARGET}-unknown-uefi"
rustup target add ${UEFI_TARGET} --toolchain nightly
cargo +nightly build --release --target ${UEFI_TARGET} --features uefi -p stub
cp target/${UEFI_TARGET}/release/stub.efi /stub.efi
EOF

# ============================================================
FROM scratch AS stub-package

COPY --link --from=builder /stub.efi /stub.efi

LABEL org.opencontainers.image.title="stub"
LABEL org.opencontainers.image.description="Muak UEFI stub"
LABEL org.opencontainers.image.source="https://github.com/Sawangg/muak"
