FROM alpine:latest AS kernel-build

ARG KERNEL_VERSION=6.17.8

RUN apk add --no-cache \
  build-base \
  bc \
  bison \
  flex \
  linux-headers \
  elfutils-dev \
  openssl-dev \
  perl \
  python3 \
  xz

WORKDIR /src

RUN wget -q "https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-${KERNEL_VERSION}.tar.xz" && \
  tar -xJf linux-${KERNEL_VERSION}.tar.xz --strip-components=1 && \
  rm linux-${KERNEL_VERSION}.tar.xz && \
  make mrproper

COPY kernel.config .config

RUN make -j$(nproc)

RUN cp arch/x86/boot/bzImage /bzImage

# ============================================================
FROM scratch AS kernel-package

COPY --from=kernel-build /bzImage /bzImage

LABEL org.opencontainers.image.title="muak-kernel"
LABEL org.opencontainers.image.description="Muak Linux kernel package"
LABEL org.opencontainers.image.source="https://github.com/Sawangg/muak"
