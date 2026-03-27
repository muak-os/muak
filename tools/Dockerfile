# syntax = docker/dockerfile-upstream:1.22.0-labs

ARG RUST_VERSION=1.94
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
  apk add --no-cache --no-scripts musl-dev

COPY Cargo.toml Cargo.lock ./
RUN sed -i '/members = \[/,/\]/c\members = ["tools/imager", "tools/ramune", "tools/yuki", "libs/erofs"]' Cargo.toml

COPY tools/imager ./tools/imager
COPY tools/ramune ./tools/ramune
COPY tools/yuki   ./tools/yuki
COPY libs/erofs   ./libs/erofs

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/build/target \
  <<EOF
set -euo pipefail

RUST_TARGET="${TARGET}-unknown-linux-musl"
cargo build --release --target ${RUST_TARGET} -p imager -p ramune -p yuki
cp target/${RUST_TARGET}/release/imager /imager
cp target/${RUST_TARGET}/release/ramune /ramune
cp target/${RUST_TARGET}/release/yuki   /yuki
EOF

# ─────────────────────────────────────────────────────────────────────────────
FROM scratch

COPY --link --from=builder /imager /imager
COPY --link --from=builder /ramune /ramune
COPY --link --from=builder /yuki   /yuki

LABEL org.opencontainers.image.title="tools"
LABEL org.opencontainers.image.description="Muak build tools"
LABEL org.opencontainers.image.source="https://github.com/muak-os/muak"
