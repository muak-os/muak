#!/bin/bash
set -euo pipefail

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

ARCH="${ARCH:-x86_64}"
KERNEL_DIR="$(pwd)/output/${ARCH}"
INITRAMFS_FILE="${KERNEL_DIR}/initramfs.img"
CMDLINE_FILE="$(pwd)/config/cmdline.txt"
OUTPUT_DIR="$(pwd)/output/${ARCH}"

echo -e "${GREEN}==== Muak UKI Build ====${NC}"
echo -e "${GREEN}Architecture: ${ARCH}${NC}"
echo -e "${GREEN}Kernel: ${KERNEL_DIR}/vmlinuz-*${NC}"
echo -e "${GREEN}Initramfs: ${INITRAMFS_FILE}${NC}"
echo -e "${GREEN}Cmdline: ${CMDLINE_FILE}${NC}"
echo

if [ ! -f "${INITRAMFS_FILE}" ]; then
    echo -e "${RED}ERROR: initramfs not found at ${INITRAMFS_FILE}${NC}"
    exit 1
fi

if [ ! -f "${CMDLINE_FILE}" ]; then
    echo -e "${RED}ERROR: cmdline not found at ${CMDLINE_FILE}${NC}"
    exit 1
fi

KERNEL_IMAGE=$(ls ${KERNEL_DIR}/vmlinuz-* 2>/dev/null | head -1)
if [ -z "${KERNEL_IMAGE}" ]; then
    echo -e "${RED}ERROR: Kernel image not found in ${KERNEL_DIR}${NC}"
    exit 1
fi

echo -e "${GREEN}Found kernel: ${KERNEL_IMAGE}${NC}"

if [ "${ARCH}" = "arm64" ]; then
    UKI_OUTPUT="${OUTPUT_DIR}/muak-arm64.efi"
else
    UKI_OUTPUT="${OUTPUT_DIR}/muak-x86_64.efi"
fi

if ! command -v ukify &> /dev/null; then
    echo -e "${RED}ERROR: ukify not found${NC}"
    echo -e "${YELLOW}Install systemd-ukify package${NC}"
    exit 1
fi

echo -e "${YELLOW}Building UKI with ukify...${NC}"
ukify build \
    --linux="${KERNEL_IMAGE}" \
    --initrd="${INITRAMFS_FILE}" \
    --cmdline="@${CMDLINE_FILE}" \
    --output="${UKI_OUTPUT}"

echo
echo -e "${GREEN}==== UKI Build Complete ====${NC}"
echo -e "${GREEN}UKI: ${UKI_OUTPUT}${NC}"
ls -lh "${UKI_OUTPUT}"
