#!/bin/bash
set -euo pipefail

ARCH="${ARCH:-x86_64}"
KERNEL_VERSION="${KERNEL_VERSION:-6.15.11}"
BUILD_DIR="$(pwd)/build/kernel/${ARCH}"
CONFIG_FILE="$(pwd)/kernel/config/${ARCH}/kernel.config"

echo "==== Muak Kernel Build ===="
echo "Architecture: ${ARCH}"
echo "Kernel Version: ${KERNEL_VERSION}"
echo "Build Directory: ${BUILD_DIR}"

mkdir -p "${BUILD_DIR}"

cd "${BUILD_DIR}"

if [ ! -f "linux-${KERNEL_VERSION}.tar.xz" ]; then
    echo "Downloading Linux kernel ${KERNEL_VERSION}..."
    wget "https://cdn.kernel.org/pub/linux/kernel/v${KERNEL_VERSION%%.*}.x/linux-${KERNEL_VERSION}.tar.xz"
fi

if [ ! -d "linux-${KERNEL_VERSION}" ]; then
    echo "Extracting kernel source..."
    tar -xf "linux-${KERNEL_VERSION}.tar.xz"
fi

cd "linux-${KERNEL_VERSION}"

if [ -f "${CONFIG_FILE}" ]; then
    echo "Using custom kernel config from ${CONFIG_FILE}"
    cp "${CONFIG_FILE}" .config
else
    echo "ERROR: Config file not found at ${CONFIG_FILE}"
    exit 1
fi

if [ "${ARCH}" = "arm64" ]; then
    export ARCH=arm64
    MAKE_ARCH="ARCH=arm64"
else
    export ARCH=x86_64
    MAKE_ARCH="ARCH=x86_64"
fi

echo "Running olddefconfig to resolve config dependencies..."
make ${MAKE_ARCH} olddefconfig

echo "Building kernel..."
make ${MAKE_ARCH} -j$(nproc)

echo "==== Build Complete ===="
if [ "${ARCH}" = "arm64" ]; then
    echo "Kernel: ${BUILD_DIR}/linux-${KERNEL_VERSION}/arch/arm64/boot/Image"
else
    echo "Kernel: ${BUILD_DIR}/linux-${KERNEL_VERSION}/arch/x86/boot/bzImage"
fi
echo "Config: ${BUILD_DIR}/linux-${KERNEL_VERSION}/.config"
echo "System.map: ${BUILD_DIR}/linux-${KERNEL_VERSION}/System.map"
