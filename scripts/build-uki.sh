#!/bin/bash
set -euo pipefail

ARCH="${ARCH:-x86_64}"
KERNEL_DIR="$(pwd)/output/${ARCH}"
INITRAMFS_FILE="${KERNEL_DIR}/initramfs.cpio.gz"
CMDLINE_FILE="$(pwd)/initramfs/cmdline.txt"
OUTPUT_DIR="$(pwd)/output/${ARCH}"

echo "==== Muak UKI Build ===="
echo "Architecture: ${ARCH}"
echo "Kernel: ${KERNEL_DIR}/vmlinuz-*"
echo "Initramfs: ${INITRAMFS_FILE}"
echo "Cmdline: ${CMDLINE_FILE}"

if [ ! -f "${INITRAMFS_FILE}" ]; then
    echo "ERROR: initramfs not found at ${INITRAMFS_FILE}"
    exit 1
fi

KERNEL_IMAGE=$(ls ${KERNEL_DIR}/vmlinuz-* 2>/dev/null | head -1)
if [ -z "${KERNEL_IMAGE}" ]; then
    echo "ERROR: Kernel image not found in ${KERNEL_DIR}"
    exit 1
fi

echo "Found kernel: ${KERNEL_IMAGE}"

if [ "${ARCH}" = "arm64" ]; then
    STUB_PATH="/usr/lib/systemd/boot/efi/linuxaa64.efi.stub"
    UKI_OUTPUT="${OUTPUT_DIR}/muak-arm64.efi"
else
    STUB_PATH="/usr/lib/systemd/boot/efi/linuxx64.efi.stub"
    UKI_OUTPUT="${OUTPUT_DIR}/muak-x86_64.efi"
fi

if command -v ukify &> /dev/null; then
    echo "Using ukify to build UKI..."
    ukify build \
        --linux="${KERNEL_IMAGE}" \
        --initrd="${INITRAMFS_FILE}" \
        --cmdline="@${CMDLINE_FILE}" \
        --output="${UKI_OUTPUT}"
elif [ -f "${STUB_PATH}" ]; then
    echo "Using objcopy to build UKI..."

    OSREL_FILE="${OUTPUT_DIR}/os-release"
    cat > "${OSREL_FILE}" << EOF
NAME=Muak
ID=muak
PRETTY_NAME="Muak Linux"
VERSION_ID=0.1.0
HOME_URL=https://github.com/yourusername/muak
BUG_REPORT_URL=https://github.com/yourusername/muak/issues
EOF

    objcopy \
        --add-section .osrel="${OSREL_FILE}" --change-section-vma .osrel=0x20000 \
        --add-section .cmdline="${CMDLINE_FILE}" --change-section-vma .cmdline=0x30000 \
        --add-section .linux="${KERNEL_IMAGE}" --change-section-vma .linux=0x2000000 \
        --add-section .initrd="${INITRAMFS_FILE}" --change-section-vma .initrd=0x3000000 \
        "${STUB_PATH}" "${UKI_OUTPUT}"
else
    echo "ERROR: Neither ukify nor EFI stub found"
    echo "Install systemd-ukify or systemd-boot-efi"
    exit 1
fi

echo "==== UKI Build Complete ===="
echo "UKI: ${UKI_OUTPUT}"
ls -lh "${UKI_OUTPUT}"
