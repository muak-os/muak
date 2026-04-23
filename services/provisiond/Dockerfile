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

RUN sed -i '/members = \[/,/\]/c\members = ["services/provisiond", "core/granola", "tools/imager", "libs/kmsg", "tools/yuki", "libs/config", "libs/pki", "libs/sbolt", "libs/luks2"]' Cargo.toml && \
  cargo chef prepare --recipe-path recipe.json

# ─────────────────────────────────────────────────────────────────────────────
FROM deps AS builder

COPY --from=planner /build/recipe.json recipe.json

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/usr/local/cargo/git \
  --mount=type=cache,target=/build/target \
  cargo chef cook --release --target ${RUST_TARGET} --recipe-path recipe.json

COPY api ./api
COPY libs/config/default.toml ./libs/config/default.toml
COPY services/provisiond ./services/provisiond
COPY core/granola ./core/granola
COPY libs/kmsg ./libs/kmsg
COPY tools/yuki ./tools/yuki
COPY tools/imager ./tools/imager
COPY libs/config ./libs/config
COPY libs/pki ./libs/pki
COPY libs/sbolt ./libs/sbolt
COPY libs/luks2 ./libs/luks2
COPY core/kernel/cmdline-amd64.txt ./core/kernel/cmdline-amd64.txt
COPY core/kernel/cmdline-arm64.txt ./core/kernel/cmdline-arm64.txt

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/usr/local/cargo/git \
  --mount=type=cache,target=/build/target \
  <<EOF
set -euo pipefail

cargo build --release --target ${RUST_TARGET} --locked -p provisiond
cp target/${RUST_TARGET}/release/provisiond /provisiond
EOF

# ─────────────────────────────────────────────────────────────────────────────
FROM scratch

COPY --link --from=builder /provisiond                            /provisiond
COPY --link                services/provisiond/provisiond.service /etc/services/provisiond.service

LABEL org.opencontainers.image.title="provisiond"
LABEL org.opencontainers.image.description="Muak provisioning daemon"
LABEL org.opencontainers.image.source="https://github.com/muak-os/muak"
