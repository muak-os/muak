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
RUN sed -i '/members = \[/,/\]/c\members = ["internal/apid", "internal/shared/kmsg", "internal/shared/notify", "internal/shared/sysconfig"]' Cargo.toml

COPY api ./api
COPY internal/default.toml ./internal/default.toml
COPY internal/apid ./internal/apid
COPY internal/shared/kmsg ./internal/shared/kmsg
COPY internal/shared/notify ./internal/shared/notify
COPY internal/shared/sysconfig ./internal/shared/sysconfig

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/build/target \
  <<EOF
set -euo pipefail
cargo build --release --target ${TARGET} -p apid
cp target/${TARGET}/release/apid /apid
EOF

# ============================================================
FROM scratch AS apid-package

COPY --link --from=builder /apid /apid

LABEL org.opencontainers.image.title="apid"
LABEL org.opencontainers.image.description="Muak external gRPC API daemon"
LABEL org.opencontainers.image.source="https://github.com/Sawangg/muak"
