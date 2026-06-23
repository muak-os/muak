# syntax = docker/dockerfile-upstream:1.22.0-labs

ARG ALPINE_VERSION
ARG KERNEL_VERSION=7.1.1

# ─────────────────────────────────────────────────────────────────────────────
FROM docker.io/alpine:${ALPINE_VERSION} AS kernel-build

ARG KERNEL_VERSION
ARG TARGETARCH

RUN --mount=type=cache,target=/var/cache/apk \
  --mount=type=cache,target=/etc/apk/cache \
  apk add --no-cache --no-scripts \
  bc \
  bison \
  ccache \
  clang \
  diffutils \
  elfutils-dev \
  findutils \
  flex \
  gawk \
  grep \
  kmod \
  lld \
  linux-headers \
  llvm \
  make \
  musl-dev \
  openssl \
  openssl-dev \
  pahole \
  perl \
  python3 \
  sbsigntool \
  xz \
  zstd

WORKDIR /src

RUN <<EOF
set -euo pipefail
MAJOR_VERSION=$(echo "${KERNEL_VERSION}" | cut -d. -f1)
wget -q "https://cdn.kernel.org/pub/linux/kernel/v${MAJOR_VERSION}.x/linux-${KERNEL_VERSION}.tar.xz"
wget -q "https://cdn.kernel.org/pub/linux/kernel/v${MAJOR_VERSION}.x/sha256sums.asc"
grep "linux-${KERNEL_VERSION}.tar.xz" sha256sums.asc | sha256sum -c -
rm sha256sums.asc
tar -xJf linux-${KERNEL_VERSION}.tar.xz --strip-components=1
rm linux-${KERNEL_VERSION}.tar.xz
mkdir -p certs
make mrproper
EOF

COPY core/kernel/config-${TARGETARCH} .config
COPY core/kernel/kernel-signing-cert.pem certs/kernel-signing-cert.pem

RUN make LLVM=1 olddefconfig

RUN --mount=type=cache,target=/root/.ccache \
  export PATH="/usr/lib/ccache/bin:$PATH" && \
  make -j$(nproc) LLVM=1 CC="ccache clang" LD=ld.lld && \
  make -j$(nproc) LLVM=1 CC="ccache clang" LD=ld.lld modules

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

# ─────────────────────────────────────────────────────────────────────────────
FROM kernel-build AS modules-install

ARG KERNEL_VERSION

RUN make LLVM=1 INSTALL_MOD_PATH=/modules INSTALL_MOD_STRIP=1 modules_install

RUN make -s LLVM=1 kernelrelease > /kernel-release

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

# ─────────────────────────────────────────────────────────────────────────────
FROM docker.io/alpine:${ALPINE_VERSION} AS kernel-export

ARG TARGETARCH

COPY --link --from=kernel-build /src/arch /src/arch
COPY core/kernel/cmdline-${TARGETARCH}.txt /src/cmdline

RUN <<EOF
set -euo pipefail
if [ "${TARGETARCH}" = "arm64" ]; then
  cp /src/arch/arm64/boot/Image /vmlinuz
else
  cp /src/arch/x86/boot/bzImage /vmlinuz
fi

cp /src/cmdline /cmdline
EOF

# ─────────────────────────────────────────────────────────────────────────────
FROM scratch

COPY --link --from=kernel-export   /vmlinuz             /vmlinuz
COPY --link --from=kernel-export   /cmdline             /cmdline
COPY --link --from=modules-install /modules/lib/modules /lib/modules

LABEL org.opencontainers.image.title="muak-kernel"
LABEL org.opencontainers.image.description="Muak Linux kernel package"
LABEL org.opencontainers.image.source="https://github.com/muak-os/muak"
