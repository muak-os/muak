# syntax = docker/dockerfile-upstream:1.20.0-labs

ARG ALPINE_VERSION=3.23
ARG KERNEL_VERSION=6.18.1

# ============================================================
FROM docker.io/alpine:${ALPINE_VERSION} AS kernel-build

ARG KERNEL_VERSION

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
  xz \
  zstd

WORKDIR /src

RUN wget -q "https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-${KERNEL_VERSION}.tar.xz" && \
  tar -xJf linux-${KERNEL_VERSION}.tar.xz --strip-components=1 && \
  rm linux-${KERNEL_VERSION}.tar.xz && \
  mkdir -p certs && \
  make mrproper

COPY pkgs/kernel/config-amd64 .config
COPY pkgs/kernel/kernel-signing-cert.pem certs/kernel-signing-cert.pem

RUN make olddefconfig

RUN --mount=type=cache,target=/root/.ccache \
  export PATH="/usr/lib/ccache/bin:$PATH" && \
  make -j$(nproc) CC="ccache gcc" && \
  make -j$(nproc) CC="ccache gcc" modules

RUN --mount=type=secret,id=kernel_key,target=/run/secrets/kernel-signing-key.pem <<EOF
set -euo pipefail
if [ -f /run/secrets/kernel-signing-key.pem ]; then
  ./scripts/sign-file sha512 \
    /run/secrets/kernel-signing-key.pem \
    certs/kernel-signing-cert.pem \
    arch/x86/boot/bzImage
  echo "bzImage signed successfully"
else
  echo "No signing key provided, bzImage is unsigned"
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
FROM scratch AS kernel-package

COPY --from=kernel-build /src/arch/x86/boot/bzImage /bzImage
COPY --from=modules-install /modules/lib/modules /lib/modules

LABEL org.opencontainers.image.title="muak-kernel"
LABEL org.opencontainers.image.description="Muak Linux kernel package"
LABEL org.opencontainers.image.source="https://github.com/Sawangg/muak"
