# syntax = docker/dockerfile-upstream:1.20.0-labs

ARG ALPINE_VERSION=3.23
ARG KERNEL_VERSION=6.18.4

# ============================================================
FROM docker.io/alpine:${ALPINE_VERSION} AS kernel-build

ARG KERNEL_VERSION
ARG TARGETARCH

RUN apk add --no-cache \
  argp-standalone \
  bc \
  bison \
  build-base \
  ccache \
  coreutils \
  diffutils \
  elfutils-dev \
  findutils \
  flex \
  gawk \
  gmp-dev \
  kmod \
  linux-headers \
  mpc1-dev \
  mpfr-dev \
  openssl \
  openssl-dev \
  pahole \
  perl \
  python3 \
  sbsigntool \
  xz \
  zstd

WORKDIR /src

RUN wget -q "https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-${KERNEL_VERSION}.tar.xz" && \
  tar -xJf linux-${KERNEL_VERSION}.tar.xz --strip-components=1 && \
  rm linux-${KERNEL_VERSION}.tar.xz && \
  mkdir -p certs && \
  make mrproper

COPY pkgs/kernel/config-${TARGETARCH} .config
COPY pkgs/kernel/kernel-signing-cert.pem certs/kernel-signing-cert.pem

RUN make olddefconfig

RUN --mount=type=cache,target=/root/.ccache \
  export PATH="/usr/lib/ccache/bin:$PATH" && \
  make -j$(nproc) CC="ccache gcc" && \
  make -j$(nproc) CC="ccache gcc" modules

RUN --mount=type=secret,id=kernel_key,target=/run/secrets/kernel-signing-key.pem <<EOF
set -euo pipefail
if [ "${TARGETARCH}" = "arm64" ]; then
  KERNEL_PATH="arch/arm64/boot/Image"
else
  KERNEL_PATH="arch/x86/boot/bzImage"
fi

if [ -f /run/secrets/kernel-signing-key.pem ]; then
  sbsign --key /run/secrets/kernel-signing-key.pem \
    --cert certs/kernel-signing-cert.pem \
    --output ${KERNEL_PATH}.signed \
    ${KERNEL_PATH}
  mv ${KERNEL_PATH}.signed ${KERNEL_PATH}
  echo "Kernel signed with PE signature successfully"
else
  echo "No signing key provided, kernel is unsigned"
fi
EOF

# ============================================================
FROM kernel-build AS modules-install

ARG KERNEL_VERSION

RUN make INSTALL_MOD_PATH=/modules INSTALL_MOD_STRIP=1 modules_install

RUN make -s kernelrelease > /kernel-release

RUN --mount=type=cache,target=/root/.ccache <<EOF
set -euo pipefail
KREL=$(cat /kernel-release)
MODDIR="/modules/lib/modules/${KREL}"

rm -f "${MODDIR}/build" "${MODDIR}/source"

find "${MODDIR}" -name '*.ko' -exec zstd -19 --rm -q {} \;

depmod -a -b /modules "${KREL}"

cd "${MODDIR}"
rm -f modules.builtin.modinfo modules.devname modules.order 2>/dev/null || true
EOF

# ============================================================
FROM docker.io/alpine:${ALPINE_VERSION} AS kernel-export

ARG TARGETARCH

COPY --from=kernel-build /src/arch /src/arch

RUN <<EOF
set -euo pipefail
if [ "${TARGETARCH}" = "arm64" ]; then
  cp /src/arch/arm64/boot/Image /vmlinuz
else
  cp /src/arch/x86/boot/bzImage /vmlinuz
fi
EOF

# ============================================================
FROM scratch AS kernel-package

COPY --from=kernel-export /vmlinuz /vmlinuz
COPY --from=modules-install /modules/lib/modules /lib/modules

LABEL org.opencontainers.image.title="muak-kernel"
LABEL org.opencontainers.image.description="Muak Linux kernel package"
LABEL org.opencontainers.image.source="https://github.com/Sawangg/muak"
