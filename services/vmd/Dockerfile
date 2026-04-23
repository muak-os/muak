# syntax = docker/dockerfile-upstream:1.22.0-labs

ARG RUST_VERSION
ARG ALPINE_VERSION
ARG SOURCE_DATE_EPOCH=0

# ─────────────────────────────────────────────────────────────────────────────
FROM docker.io/rust:${RUST_VERSION}-alpine${ALPINE_VERSION} AS deps

ARG SOURCE_DATE_EPOCH
ARG TARGETARCH

ARG TARGET=${TARGETARCH/amd64/x86_64}
ARG TARGET=${TARGET/arm64/aarch64}

ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}
ENV RUST_TARGET=${TARGET}-unknown-linux-musl

WORKDIR /build

RUN --mount=type=cache,target=/var/cache/apk \
  --mount=type=cache,target=/etc/apk/cache \
  apk add --no-cache --no-scripts musl-dev

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/usr/local/cargo/git \
  cargo install --locked cargo-chef

# ─────────────────────────────────────────────────────────────────────────────
FROM deps AS planner

COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/usr/local/cargo/git \
  cargo chef prepare --bin vmd --recipe-path recipe.json

# ─────────────────────────────────────────────────────────────────────────────
FROM deps AS builder

COPY --from=planner /build/recipe.json recipe.json

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/usr/local/cargo/git \
  --mount=type=cache,target=/build/target \
  cargo chef cook --release --target ${RUST_TARGET} --package vmd --recipe-path recipe.json

COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/usr/local/cargo/git \
  --mount=type=cache,target=/build/target \
  <<EOF
set -euo pipefail

cargo build --release --target ${RUST_TARGET} --locked -p vmd
cp target/${RUST_TARGET}/release/vmd /vmd
EOF

# ─────────────────────────────────────────────────────────────────────────────
FROM scratch

COPY --link --from=builder /vmd                            /vmd
COPY --link                services/vmd/vmd.service        /etc/services/vmd.service

LABEL org.opencontainers.image.title="vmd"
LABEL org.opencontainers.image.description="Muak VM management daemon"
LABEL org.opencontainers.image.source="https://github.com/muak-os/muak"
