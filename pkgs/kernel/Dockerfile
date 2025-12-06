FROM alpine:latest AS kernel-build

ARG KERNEL_VERSION=6.18

RUN apk add --no-cache \
  build-base \
  bc \
  bison \
  coreutils \
  diffutils \
  findutils \
  flex \
  gawk \
  gmp-dev \
  mpc1-dev \
  mpfr-dev \
  linux-headers \
  elfutils-dev \
  openssl \
  openssl-dev \
  pahole \
  perl \
  python3 \
  xz \
  zstd \
  argp-standalone

WORKDIR /src

RUN wget -q "https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-${KERNEL_VERSION}.tar.xz" && \
  tar -xJf linux-${KERNEL_VERSION}.tar.xz --strip-components=1 && \
  rm linux-${KERNEL_VERSION}.tar.xz && \
  make mrproper

COPY pkgs/kernel/config-amd64 .config

RUN make olddefconfig && make -j$(nproc)

RUN cp arch/x86/boot/bzImage /bzImage

# ============================================================
FROM scratch AS kernel-package

COPY --from=kernel-build /bzImage /bzImage

LABEL org.opencontainers.image.title="muak-kernel"
LABEL org.opencontainers.image.description="Muak Linux kernel package"
LABEL org.opencontainers.image.source="https://github.com/Sawangg/muak"
