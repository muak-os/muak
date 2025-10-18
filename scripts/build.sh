#!/bin/bash
set -euo pipefail

ARCH="${ARCH:-x86_64}"
KERNEL_VERSION="${KERNEL_VERSION:-6.6.58}"
BUILD_DIR="$(pwd)/build/${ARCH}"
OUTPUT_DIR="$(pwd)/output/${ARCH}"
CONFIG_FILE="$(pwd)/kernel/config/${ARCH}/kernel.config"

echo "==== Muak Kernel Build ===="
echo "Architecture: ${ARCH}"
echo "Kernel Version: ${KERNEL_VERSION}"
echo "Build Directory: ${BUILD_DIR}"
echo "Output Directory: ${OUTPUT_DIR}"

mkdir -p "${BUILD_DIR}"
mkdir -p "${OUTPUT_DIR}"

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
    export CROSS_COMPILE=aarch64-linux-gnu-
    MAKE_ARCH="ARCH=arm64 CROSS_COMPILE=aarch64-linux-gnu-"
else
    export ARCH=x86_64
    MAKE_ARCH="ARCH=x86_64"
fi

echo "Running olddefconfig to resolve config dependencies..."
make ${MAKE_ARCH} olddefconfig

echo "Building kernel..."
make ${MAKE_ARCH} -j$(nproc)

echo "Copying artifacts to output directory..."
if [ "${ARCH}" = "arm64" ]; then
    cp arch/arm64/boot/Image "${OUTPUT_DIR}/vmlinuz-${KERNEL_VERSION}"
else
    cp arch/x86/boot/bzImage "${OUTPUT_DIR}/vmlinuz-${KERNEL_VERSION}"
fi

cp .config "${OUTPUT_DIR}/config-${KERNEL_VERSION}"
cp System.map "${OUTPUT_DIR}/System.map-${KERNEL_VERSION}"

echo "==== Build Complete ===="
echo "Kernel: ${OUTPUT_DIR}/vmlinuz-${KERNEL_VERSION}"
echo "Config: ${OUTPUT_DIR}/config-${KERNEL_VERSION}"
echo "System.map: ${OUTPUT_DIR}/System.map-${KERNEL_VERSION}"
