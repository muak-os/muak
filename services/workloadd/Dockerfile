# syntax = docker/dockerfile-upstream:1.22.0-labs

ARG RUST_IMAGE
ARG SOURCE_DATE_EPOCH=0

# ─────────────────────────────────────────────────────────────────────────────
FROM ${RUST_IMAGE} AS deps

ARG SOURCE_DATE_EPOCH
ARG TARGETARCH

ARG TARGET=${TARGETARCH/amd64/x86_64}
ARG TARGET=${TARGET/arm64/aarch64}

ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}
ENV RUST_TARGET=${TARGET}-unknown-linux-musl

WORKDIR /build

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/usr/local/cargo/git \
  cargo install --locked cargo-chef@0.1.78

COPY rust-toolchain.toml .
RUN rustup toolchain install --profile minimal

# ─────────────────────────────────────────────────────────────────────────────
FROM deps AS planner

COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/usr/local/cargo/git \
  cargo chef prepare --bin workloadd --recipe-path recipe.json

# ─────────────────────────────────────────────────────────────────────────────
FROM deps AS builder

COPY --from=planner /build/recipe.json recipe.json

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/usr/local/cargo/git \
  --mount=type=cache,target=/build/target \
  cargo chef cook --release --target ${RUST_TARGET} --package workloadd --recipe-path recipe.json

COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/usr/local/cargo/git \
  --mount=type=cache,target=/build/target \
  <<EOF
set -euo pipefail

cargo build --release --target ${RUST_TARGET} --locked -p workloadd
cp target/${RUST_TARGET}/release/workloadd /workloadd
EOF

# ─────────────────────────────────────────────────────────────────────────────
FROM scratch

COPY --link --from=builder /workloadd                            /workloadd
COPY --link                services/workloadd/workloadd.service        /etc/services/workloadd.service

LABEL org.opencontainers.image.title="workloadd"
LABEL org.opencontainers.image.description="Muak VM management daemon"
LABEL org.opencontainers.image.source="https://github.com/muak-os/muak"
