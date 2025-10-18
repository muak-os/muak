#!/bin/bash
set -euo pipefail

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

ARCH="${ARCH:-x86_64}"
KERNEL_VERSION="${KERNEL_VERSION:-6.6.58}"
KERNEL_DIR="$(pwd)/build/${ARCH}/linux-${KERNEL_VERSION}"
CONFIG_FILE="$(pwd)/config/mkinitcpio.conf"
OUTPUT_DIR="$(pwd)/output/${ARCH}"

echo -e "${GREEN}==== Muak initramfs build====${NC}"
echo -e "${GREEN}Architecture: ${ARCH}${NC}"
echo -e "${GREEN}Kernel Version: ${KERNEL_VERSION}${NC}"
echo -e "${GREEN}Kernel Directory: ${KERNEL_DIR}${NC}"
echo -e "${GREEN}Config: ${CONFIG_FILE}${NC}"
echo -e "${GREEN}Output: ${OUTPUT_DIR}/initramfs.img${NC}"
echo

if [ ! -f "${CONFIG_FILE}" ]; then
    echo -e "${RED}ERROR: mkinitcpio config not found at ${CONFIG_FILE}${NC}"
    exit 1
fi

if [ ! -d "${KERNEL_DIR}" ]; then
    echo -e "${RED}ERROR: Kernel source directory not found at ${KERNEL_DIR}${NC}"
    echo -e "${YELLOW}Please run build-kernel.sh first${NC}"
    exit 1
fi

mkdir -p "${OUTPUT_DIR}"

echo -e "${YELLOW}Installing kernel modules to temporary location...${NC}"
MODULES_DIR="$(pwd)/build/${ARCH}/modules"
rm -rf "${MODULES_DIR}"
make -C "${KERNEL_DIR}" INSTALL_MOD_PATH="${MODULES_DIR}" modules_install

echo -e "${YELLOW}Generating initramfs with mkinitcpio...${NC}"
mkinitcpio \
    --config "${CONFIG_FILE}" \
    --kernel "${KERNEL_VERSION}" \
    --moduleroot "${MODULES_DIR}" \
    --generate "${OUTPUT_DIR}/initramfs.img"

echo
echo -e "${GREEN}==== initramfs Build Complete ====${NC}"
echo -e "${GREEN}initramfs: ${OUTPUT_DIR}/initramfs.img${NC}"
echo
ls -lh "${OUTPUT_DIR}/initramfs.img"
