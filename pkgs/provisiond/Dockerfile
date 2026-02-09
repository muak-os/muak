# syntax = docker/dockerfile-upstream:1.20.0-labs

ARG RUST_VERSION=1.93.0
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
  apk add --no-cache --no-scripts musl-dev protoc

COPY Cargo.toml Cargo.lock ./
RUN sed -i '/members = \[/,/\]/c\members = ["internal/provisiond", "internal/imager", "internal/shared/kmsg", "internal/shared/notify", "internal/yuki", "internal/shared/sysconfig", "internal/shared/pki", "internal/sbolt"]' Cargo.toml

COPY api ./api
COPY internal/default.toml ./internal/default.toml
COPY internal/provisiond ./internal/provisiond
COPY internal/shared/kmsg ./internal/shared/kmsg
COPY internal/shared/notify ./internal/shared/notify
COPY internal/yuki ./internal/yuki
COPY internal/imager ./internal/imager
COPY internal/shared/sysconfig ./internal/shared/sysconfig
COPY internal/shared/pki ./internal/shared/pki
COPY internal/sbolt ./internal/sbolt

COPY pkgs/kernel/cmdline-amd64.txt ./pkgs/kernel/cmdline-amd64.txt
COPY pkgs/kernel/cmdline-arm64.txt ./pkgs/kernel/cmdline-arm64.txt

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/build/target \
  <<EOF
set -euo pipefail

RUST_TARGET="${TARGET}-unknown-linux-musl"
cargo build --release --target ${RUST_TARGET} -p provisiond
cp target/${RUST_TARGET}/release/provisiond /provisiond
EOF

# ============================================================
FROM scratch AS provisiond-package

COPY --link --from=builder /provisiond /provisiond

LABEL org.opencontainers.image.title="provisiond"
LABEL org.opencontainers.image.description="Muak provisioning daemon"
LABEL org.opencontainers.image.source="https://github.com/Sawangg/muak"
