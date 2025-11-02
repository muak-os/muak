#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

ARCH="${1:-x86_64}"
KERNEL_BUILD_DIR="$PROJECT_ROOT/build/kernel/${ARCH}"
INITRAMFS_FILE="$PROJECT_ROOT/build/initramfs.img"
CMDLINE_FILE="$PROJECT_ROOT/config/cmdline.txt"
OUTPUT_DIR="$PROJECT_ROOT/build"
STUB_FILE="$PROJECT_ROOT/config/uki/linuxx64.efi.stub"

echo -e "${GREEN}==== Muak UKI Build ====${NC}"
echo -e "${GREEN}Architecture: ${ARCH}${NC}"
echo -e "${GREEN}Kernel Build Dir: ${KERNEL_BUILD_DIR}${NC}"
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

# Find kernel bzImage in build tree
BZIMAGE=$(find "${KERNEL_BUILD_DIR}" -path "*/arch/x86/boot/bzImage" -o -path "*/arch/x86_64/boot/bzImage" 2>/dev/null | head -1)
if [ -z "${BZIMAGE}" ]; then
    echo -e "${RED}ERROR: bzImage not found in ${KERNEL_BUILD_DIR}${NC}"
    exit 1
fi

echo -e "${GREEN}Found kernel bzImage: ${BZIMAGE}${NC}"

if ! command -v llvm-objcopy &> /dev/null; then
    echo -e "${RED}ERROR: llvm-objcopy not found${NC}"
    echo -e "${RED}Install llvm package${NC}"
    exit 1
fi

if [ ! -f "${STUB_FILE}" ]; then
    echo -e "${RED}ERROR: EFI stub not found at ${STUB_FILE}${NC}"
    exit 1
fi

mkdir -p "${OUTPUT_DIR}"

UKI_OUTPUT="${OUTPUT_DIR}/muak-${ARCH}.efi"

echo -e "${YELLOW}Building UKI with llvm-objcopy...${NC}"

# Create os-release file
OS_RELEASE_FILE="${OUTPUT_DIR}/os-release.tmp"
cat > "${OS_RELEASE_FILE}" << EOF
ID=muak
NAME=Muak Linux
PRETTY_NAME=Muak Linux
VERSION_ID=0.1.0
BUILD_ID=$(date +%Y%m%d)
EOF

# Copy EFI stub as base
cp "${STUB_FILE}" "${UKI_OUTPUT}"

# Create uname file with kernel version
UNAME_FILE="${OUTPUT_DIR}/uname.tmp"
echo -n "6.15.11" > "${UNAME_FILE}"

llvm-objcopy \
    --add-section .osrel="${OS_RELEASE_FILE}" \
    --set-section-flags .osrel=alloc,readonly \
    --add-section .cmdline="${CMDLINE_FILE}" \
    --set-section-flags .cmdline=alloc,readonly \
    --add-section .uname="${UNAME_FILE}" \
    --set-section-flags .uname=alloc,readonly \
    --add-section .linux="${BZIMAGE}" \
    --set-section-flags .linux=alloc,readonly,code \
    --add-section .initrd="${INITRAMFS_FILE}" \
    --set-section-flags .initrd=alloc,readonly \
    "${UKI_OUTPUT}"

# Clean up temporary files
rm -f "${OS_RELEASE_FILE}" "${UNAME_FILE}"

echo
echo -e "${GREEN}==== UKI Build Complete ====${NC}"
echo -e "${GREEN}UKI: ${UKI_OUTPUT}${NC}"
ls -lh "${UKI_OUTPUT}"
