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
  cargo chef prepare --bin init --recipe-path recipe.json

# ─────────────────────────────────────────────────────────────────────────────
FROM deps AS builder

COPY --from=planner /build/recipe.json recipe.json

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/usr/local/cargo/git \
  --mount=type=cache,target=/build/target \
  cargo chef cook --release --target ${RUST_TARGET} --package millefeuille --recipe-path recipe.json

COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/usr/local/cargo/git \
  --mount=type=cache,target=/build/target \
  <<EOF
set -euo pipefail

cargo build --release --target ${RUST_TARGET} --locked -p millefeuille
cp target/${RUST_TARGET}/release/init /init
EOF

# ─────────────────────────────────────────────────────────────────────────────
FROM scratch

COPY --link --from=builder /init /init

LABEL org.opencontainers.image.title="millefeuille"
LABEL org.opencontainers.image.description="Muak stage 1 init"
LABEL org.opencontainers.image.source="https://github.com/muak-os/muak"
