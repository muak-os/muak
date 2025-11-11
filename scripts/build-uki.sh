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
BZIMAGE_CI="$PROJECT_ROOT/build/bzImage/${ARCH}/bzImage"
INITRAMFS_FILE="$PROJECT_ROOT/build/initramfs.img"
OUTPUT_DIR="$PROJECT_ROOT/build"
STUB_FILE="$OUTPUT_DIR/muak-stub-${ARCH}.efi"
CMDLINE_FILE="$OUTPUT_DIR/cmdline.txt"

echo -e "${GREEN}==== Muak UKI Build ====${NC}"
echo -e "${GREEN}Architecture: ${ARCH}${NC}"
echo -e "${GREEN}Initramfs: ${INITRAMFS_FILE}${NC}"
echo

if [ -f "${BZIMAGE_CI}" ]; then
    BZIMAGE_FILE="${BZIMAGE_CI}"
    echo -e "${GREEN}Using bzImage from CI: ${BZIMAGE_FILE}${NC}"
else
    BZIMAGE_FILE=$(find "${KERNEL_BUILD_DIR}" -path "*/arch/x86/boot/bzImage" -o -path "*/arch/x86_64/boot/bzImage" 2>/dev/null | head -1)
    if [ -z "${BZIMAGE_FILE}" ]; then
        echo -e "${RED}ERROR: bzImage not found in ${KERNEL_BUILD_DIR} or ${BZIMAGE_CI}${NC}"
        exit 1
    fi
    echo -e "${GREEN}Found kernel bzImage: ${BZIMAGE_FILE}${NC}"
fi

if [ ! -f "${INITRAMFS_FILE}" ]; then
    echo -e "${RED}ERROR: initramfs not found at ${INITRAMFS_FILE}${NC}"
    exit 1
fi

echo -n "console=ttyS0 console=tty0 init=/init" > "${CMDLINE_FILE}"
echo -e "${GREEN}Created cmdline: ${CMDLINE_FILE}${NC}"

if [ ! -f "${STUB_FILE}" ]; then
    echo -e "${RED}ERROR: EFI stub not found at ${STUB_FILE}${NC}"
    echo -e "${RED}Please run ./scripts/build-stub.sh ${ARCH} first${NC}"
    exit 1
fi

YUKI_BIN="$PROJECT_ROOT/internal/yuki/target/x86_64-unknown-linux-gnu/release/yuki"

if [ ! -f "${YUKI_BIN}" ]; then
    echo -e "${YELLOW}Building yuki (UKI builder)...${NC}"
    cd "$PROJECT_ROOT/internal/yuki"
    RUSTFLAGS='-C target-feature=+crt-static' cargo build --release --target x86_64-unknown-linux-gnu --quiet
    cd - > /dev/null
fi

if [ ! -f "${YUKI_BIN}" ]; then
    echo -e "${RED}ERROR: yuki binary not found at ${YUKI_BIN}${NC}"
    exit 1
fi

mkdir -p "${OUTPUT_DIR}"

UKI_OUTPUT="${OUTPUT_DIR}/muak-${ARCH}.efi"

echo -e "${YELLOW}Building UKI with yuki...${NC}"

"${YUKI_BIN}" \
    --stub "${STUB_FILE}" \
    --linux "${BZIMAGE_FILE}" \
    --initrd "${INITRAMFS_FILE}" \
    --cmdline "${CMDLINE_FILE}" \
    --output "${UKI_OUTPUT}"

echo
echo -e "${GREEN}==== UKI Build Complete ====${NC}"
echo -e "${GREEN}UKI: ${UKI_OUTPUT}${NC}"
ls -lh "${UKI_OUTPUT}"
