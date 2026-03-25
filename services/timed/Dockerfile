# syntax = docker/dockerfile-upstream:1.20.0-labs

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
  apk add --no-cache --no-scripts musl-dev protoc

COPY Cargo.toml Cargo.lock ./
RUN sed -i '/members = \[/,/\]/c\members = ["services/timed", "libs/kmsg", "libs/notify", "libs/config"]' Cargo.toml

COPY libs/config/default.toml ./libs/config/default.toml
COPY services/timed ./services/timed
COPY libs/kmsg ./libs/kmsg
COPY libs/notify ./libs/notify
COPY libs/config ./libs/config

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/build/target \
  <<EOF
set -euo pipefail

RUST_TARGET="${TARGET}-unknown-linux-musl"
cargo build --release --target ${RUST_TARGET} -p timed
cp target/${RUST_TARGET}/release/timed /timed
EOF

# ─────────────────────────────────────────────────────────────────────────────
FROM scratch

COPY --link --from=builder /timed /timed
COPY --link services/timed/timed.service /etc/services/timed.service

LABEL org.opencontainers.image.title="timed"
LABEL org.opencontainers.image.description="Muak time synchronization daemon"
LABEL org.opencontainers.image.source="https://github.com/muak-os/muak"
